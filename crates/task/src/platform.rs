//! Thread creation and core topology behind a trait, so consoles with fixed affinity can
//! replace both without touching the executor.

use lactose_core::{Result, ThreadClass};

use crate::span::TARGET;

/// How many cores of each kind the executor may use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CoreTopology {
    /// Performance cores. On hardware without core-type information this is the physical
    /// core count minus one.
    pub performance: u32,
    /// Efficiency cores. Zero on homogeneous CPUs, where the executor runs two low-priority
    /// background threads instead.
    pub efficiency: u32,
}

/// The class of a core: one of the two kinds [`CoreTopology`] counts.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CoreClass {
    /// A core that [`CoreTopology::performance`] counts. Where the system reports every core
    /// of the CPU as one class, every core is one.
    Performance,
    /// A core that [`CoreTopology::efficiency`] counts.
    Efficiency,
}

impl CoreTopology {
    /// The topology of a CPU that reports no core types: every physical core but one.
    fn homogeneous(physical_cores: u32) -> Self {
        Self { performance: physical_cores.saturating_sub(1).max(1), efficiency: 0 }
    }

    /// Caps the counts at the hardware threads this process may actually use, which an
    /// affinity mask or a container limit can put far below what the machine has.
    /// Performance cores stop one short of them and efficiency cores take what remains.
    fn limited_to(self, threads: u32) -> Self {
        let performance = default_frame_workers(self, Some(threads));
        Self { performance, efficiency: self.efficiency.min(threads.saturating_sub(performance)) }
    }
}

/// The default number of frame-critical workers: the performance cores, but at most one
/// fewer than the hardware threads the process may use, and at least one.
///
/// Together with [`default_background_workers`] the rule is: workers never exceed the usable
/// hardware threads; frame-critical workers stop one short and background workers take what
/// remains, so the platform thread shares a hardware thread with low-priority background
/// workers and never with a frame-critical one. The minimum of one worker of each class is
/// the only exception.
pub(crate) fn default_frame_workers(topology: CoreTopology, threads: Option<u32>) -> u32 {
    let cap = threads.map_or(u32::MAX, |threads| threads.saturating_sub(1));
    topology.performance.min(cap).max(1)
}

/// The default number of background workers beside `frame` frame-critical ones: the
/// efficiency cores, or two on a homogeneous CPU, but no more than the hardware threads the
/// frame-critical workers leave, and at least one. See [`default_frame_workers`].
pub(crate) fn default_background_workers(
    topology: CoreTopology,
    threads: Option<u32>,
    frame: u32,
) -> u32 {
    let wanted = if topology.efficiency == 0 { 2 } else { topology.efficiency };
    let cap = threads.map_or(u32::MAX, |threads| threads.saturating_sub(frame));
    wanted.min(cap).max(1)
}

/// Topology detection reduced to functions of what the operating system reported, so each
/// can be tested on any host with hand-built input.
#[allow(dead_code, reason = "each operating system uses its own part; the tests use all of it")]
mod detect {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::{CoreClass, CoreTopology};

    /// The rule that classifies physical cores by a per-core rating where a higher value
    /// means a faster core (Linux CPU capacity, the Windows efficiency class). The slowest
    /// class is the efficiency cores and every class above it is frame-critical, which on a
    /// three-class part counts the middle cores with the fastest. Ratings that are all equal
    /// carry no core-type information, and every core is then a performance core.
    pub(super) fn classifier(ratings: &[u32]) -> impl Fn(u32) -> CoreClass + use<> {
        let slowest = ratings.iter().min().copied();
        let fastest = ratings.iter().max().copied();
        move |rating| {
            if slowest != fastest && Some(rating) == slowest {
                CoreClass::Efficiency
            } else {
                CoreClass::Performance
            }
        }
    }

    /// The topology of physical cores rated as [`classifier`] reads them.
    pub(super) fn from_ratings(ratings: &[u32]) -> CoreTopology {
        let cores = ratings.len() as u32;
        if cores == 0 {
            return CoreTopology::homogeneous(1);
        }
        let class = classifier(ratings);
        let efficiency =
            ratings.iter().filter(|&&rating| class(rating) == CoreClass::Efficiency).count() as u32;
        if efficiency == 0 {
            CoreTopology::homogeneous(cores)
        } else {
            CoreTopology { performance: cores - efficiency, efficiency }
        }
    }

    /// One processor-core record of `GetLogicalProcessorInformationEx`.
    #[derive(Clone, PartialEq, Eq, Debug)]
    pub(super) struct WindowsCore {
        /// The core's efficiency class.
        pub(super) rating: u32,
        /// The core's logical processors, each as `group * 64 + number`, the key
        /// [`windows_processor`] gives a processor number.
        pub(super) processors: Vec<u32>,
    }

    /// The key of the logical processor `number` of processor group `group`.
    pub(super) fn windows_processor(group: u16, number: u8) -> u32 {
        u32::from(group) * 64 + u32::from(number)
    }

    /// Every physical core in a buffer filled by
    /// `GetLogicalProcessorInformationEx(RelationProcessorCore)`.
    ///
    /// The buffer is a sequence of variable-length records. Each starts with an eight-byte
    /// header, `Relationship: i32` then `Size: u32`, and a processor-core record carries
    /// its efficiency class in the tenth byte, the count of its group masks in the two
    /// bytes at offset 30, and from offset 32 the masks, each a `GROUP_AFFINITY` of 16
    /// bytes on 64-bit Windows: a 64-bit mask of the group's logical processors, then the
    /// group's number. A core record is 48 bytes on a machine with one processor group,
    /// shorter than the declared size of the record type, which is sized for its largest
    /// variant; only the header and the record's own `Size` say how far a record reaches.
    /// A record that would run past the buffer, or that claims a size smaller than its
    /// header, ends the walk. A core record too short for its masks still counts, with the
    /// processors that fit.
    pub(super) fn windows_cores(records: &[u8]) -> Vec<WindowsCore> {
        const HEADER: usize = 8;
        const RELATION_PROCESSOR_CORE: i32 = 0;
        const EFFICIENCY_CLASS: usize = 9;
        const GROUP_COUNT: usize = 30;
        const GROUP_MASKS: usize = 32;
        const GROUP_AFFINITY: usize = 16;

        let mut cores = Vec::new();
        let mut offset = 0;
        while let Some(header) = records.get(offset..offset + HEADER) {
            let relationship = i32::from_ne_bytes(header[..4].try_into().expect("four bytes"));
            let size = u32::from_ne_bytes(header[4..].try_into().expect("four bytes")) as usize;
            let Some(record) = records.get(offset..offset.saturating_add(size)) else { break };
            if size < HEADER {
                break;
            }
            if relationship == RELATION_PROCESSOR_CORE
                && let Some(&class) = record.get(EFFICIENCY_CLASS)
            {
                let groups = record
                    .get(GROUP_COUNT..GROUP_COUNT + 2)
                    .map_or(0, |count| u16::from_ne_bytes([count[0], count[1]]));
                let mut processors = Vec::new();
                for index in 0..usize::from(groups) {
                    let start = GROUP_MASKS + index * GROUP_AFFINITY;
                    let Some(affinity) = record.get(start..start + 10) else { break };
                    let mask = u64::from_ne_bytes(affinity[..8].try_into().expect("eight bytes"));
                    let group = u16::from_ne_bytes([affinity[8], affinity[9]]);
                    processors.extend(
                        (0..64u8)
                            .filter(|bit| mask & (1 << bit) != 0)
                            .map(|bit| windows_processor(group, bit)),
                    );
                }
                cores.push(WindowsCore { rating: u32::from(class), processors });
            }
            offset += size;
        }
        cores
    }

    /// The efficiency class of every physical core in a buffer that [`windows_cores`] walks.
    pub(super) fn windows_core_ratings(records: &[u8]) -> Vec<u32> {
        windows_cores(records).into_iter().map(|core| core.rating).collect()
    }

    /// The class of every logical processor in a buffer that [`windows_cores`] walks, by
    /// the rule of [`classifier`] over the cores' efficiency classes.
    pub(super) fn windows_processor_classes(records: &[u8]) -> BTreeMap<u32, CoreClass> {
        let cores = windows_cores(records);
        let ratings: Vec<u32> = cores.iter().map(|core| core.rating).collect();
        let class = classifier(&ratings);
        cores
            .iter()
            .flat_map(|core| {
                core.processors.iter().map(|&processor| (processor, class(core.rating)))
            })
            .collect()
    }

    /// The class of each logical CPU of an Apple machine, numbered as the kernel numbers
    /// them, from the logical CPUs of each performance level, level 0 the fastest.
    ///
    /// Level 0 is the performance class and every other level the efficiency class, as the
    /// topology counts them. macOS documents no map from a CPU number to its level: this
    /// assumes the numbering runs from the slowest level to the fastest, efficiency cores
    /// first, which is what the device tree of Apple silicon reports (`ioreg -p IODeviceTree
    /// -n cpus`: `logical-cpu-id` and `cluster-type`) on the machines it was checked on.
    pub(super) fn apple_cpu_classes(levels: &[u32]) -> BTreeMap<u32, CoreClass> {
        let mut classes = BTreeMap::new();
        let mut cpu = 0u32;
        for (level, &count) in levels.iter().enumerate().rev() {
            let class = if level == 0 { CoreClass::Performance } else { CoreClass::Efficiency };
            for _ in 0..count {
                classes.insert(cpu, class);
                cpu += 1;
            }
        }
        classes
    }

    /// One logical CPU as Linux sysfs describes it.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub(super) struct LinuxCpu {
        pub(super) cpu: u32,
        /// `topology/physical_package_id`.
        pub(super) package: u32,
        /// `topology/cluster_id`, 0 where the kernel has none (before 5.16). Device-tree
        /// arm64 kernels give every CPU package 0 and restart `core_id` in each cluster, so
        /// a core is known only by all three.
        pub(super) cluster: u32,
        /// `topology/core_id`, unique within its package and cluster.
        pub(super) core: u32,
        /// `cpu_capacity`, where the kernel exports it (arm64, RISC-V).
        pub(super) capacity: Option<u32>,
    }

    impl LinuxCpu {
        /// The physical core this CPU is a thread of.
        fn core_key(&self) -> (u32, u32, u32) {
            (self.package, self.cluster, self.core)
        }
    }

    /// Parses a sysfs CPU list such as `0-15,24,26-27`. Malformed parts are skipped.
    pub(super) fn parse_cpu_list(list: &str) -> Vec<u32> {
        let mut cpus = Vec::new();
        for part in list.trim().split(',') {
            let (first, last) = part.split_once('-').unwrap_or((part, part));
            if let (Ok(first), Ok(last)) = (first.trim().parse::<u32>(), last.trim().parse()) {
                cpus.extend(first..=last);
            }
        }
        cpus
    }

    /// Every online logical CPU under `root`, which sysfs puts at `/sys/devices/system/cpu`:
    /// a `cpu<n>` directory each, with `topology/physical_package_id`, `topology/core_id`,
    /// `topology/cluster_id` where the kernel has it, and `cpu_capacity` where it exports one.
    /// `None` where `root` cannot be read.
    pub(super) fn linux_cpus(root: &Path) -> Option<Vec<LinuxCpu>> {
        fn read_u32(path: &Path) -> Option<u32> {
            std::fs::read_to_string(path).ok()?.trim().parse().ok()
        }
        let mut cpus = Vec::new();
        for entry in std::fs::read_dir(root).ok()?.flatten() {
            let name = entry.file_name();
            let Some(cpu) = name.to_str().and_then(|name| name.strip_prefix("cpu")?.parse().ok())
            else {
                continue;
            };
            let path = entry.path();
            // An offline CPU has no topology directory.
            let (Some(package), Some(core)) = (
                read_u32(&path.join("topology/physical_package_id")),
                read_u32(&path.join("topology/core_id")),
            ) else {
                continue;
            };
            // Absent before 5.16, and -1 where the kernel knows no cluster: one cluster.
            let cluster = read_u32(&path.join("topology/cluster_id")).unwrap_or(0);
            let capacity = read_u32(&path.join("cpu_capacity"));
            cpus.push(LinuxCpu { cpu, package, cluster, core, capacity });
        }
        Some(cpus)
    }

    /// The topology of a Linux machine.
    ///
    /// Cores are rated by CPU capacity where the kernel exports it. x86 kernels do not, but
    /// on Intel hybrid parts they register two PMUs whose CPU lists name the performance
    /// cores (`/sys/devices/cpu_core/cpus`) and the efficiency cores
    /// (`/sys/devices/cpu_atom/cpus`); those are used instead. With neither there is no
    /// core-type information.
    pub(super) fn linux_topology(
        cpus: &[LinuxCpu],
        performance_cpus: Option<&str>,
        efficiency_cpus: Option<&str>,
    ) -> Option<CoreTopology> {
        let (cores, _) = linux_core_ratings(cpus, performance_cpus, efficiency_cpus);
        if cores.is_empty() {
            return None;
        }
        let ratings: Vec<u32> = cores.into_values().collect();
        Some(from_ratings(&ratings))
    }

    /// The class of each logical CPU of a Linux machine, by the ratings of
    /// [`linux_topology`]: a hardware thread has its core's class. Empty where the kernel
    /// exports neither CPU capacities nor the hybrid PMUs' lists, which say nothing of the
    /// cores' types.
    pub(super) fn linux_cpu_classes(
        cpus: &[LinuxCpu],
        performance_cpus: Option<&str>,
        efficiency_cpus: Option<&str>,
    ) -> BTreeMap<u32, CoreClass> {
        let (cores, typed) = linux_core_ratings(cpus, performance_cpus, efficiency_cpus);
        if !typed {
            return BTreeMap::new();
        }
        let ratings: Vec<u32> = cores.values().copied().collect();
        let class = classifier(&ratings);
        cpus.iter().map(|cpu| (cpu.cpu, class(cores[&cpu.core_key()]))).collect()
    }

    /// One rating per physical core, keyed by package, cluster and core id, and whether the
    /// ratings came from the kernel's capacities or the hybrid PMUs' lists rather than from
    /// nothing.
    fn linux_core_ratings(
        cpus: &[LinuxCpu],
        performance_cpus: Option<&str>,
        efficiency_cpus: Option<&str>,
    ) -> (BTreeMap<(u32, u32, u32), u32>, bool) {
        let has_capacity = cpus.iter().any(|cpu| cpu.capacity.is_some());
        let fast = performance_cpus.map(parse_cpu_list).unwrap_or_default();
        let slow = efficiency_cpus.map(parse_cpu_list).unwrap_or_default();
        let hybrid = !fast.is_empty() && !slow.is_empty();

        // Hardware threads of a core share its rating, the best of theirs.
        let mut cores = BTreeMap::new();
        for cpu in cpus {
            let rating = if has_capacity {
                cpu.capacity.unwrap_or(0)
            } else if hybrid {
                u32::from(fast.contains(&cpu.cpu))
            } else {
                0
            };
            let best = cores.entry(cpu.core_key()).or_insert(0);
            *best = rating.max(*best);
        }
        (cores, has_capacity || hybrid)
    }
}

/// A request for one thread.
#[derive(Clone, Debug)]
pub struct ThreadSpec {
    /// The thread name shown in debuggers and profilers.
    pub name: String,
    /// Scheduling class and floating-point state; the platform applies it with
    /// [`ThreadClass::enter`] or its own equivalent before running the body.
    pub class: ThreadClass,
    /// The position of this thread among those of its class, for platforms that pin.
    pub index_in_class: u32,
}

/// A thread created by a [`Platform`].
pub struct ThreadHandle {
    join: Box<dyn FnOnce() + Send>,
}

impl ThreadHandle {
    /// Wraps the function that waits for the thread to finish.
    #[must_use]
    pub fn new(join: impl FnOnce() + Send + 'static) -> Self {
        Self { join: Box::new(join) }
    }

    /// Waits for the thread to finish.
    pub fn join(self) {
        (self.join)();
    }
}

/// The operating-system services the executor needs.
pub trait Platform: Send + Sync + 'static {
    /// The cores available to the engine.
    fn topology(&self) -> CoreTopology;

    /// The hardware threads the process may run on, if the default worker counts should be
    /// capped by them: frame-critical workers at one fewer, background workers at what
    /// those leave. `None`, the default, means [`Platform::topology`] is taken as it is.
    fn hardware_threads(&self) -> Option<u32> {
        None
    }

    /// Starts a thread that runs `body` after applying `spec.class`.
    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle>;
}

/// The default platform: `std::thread`, with topology from `hw.perflevel0.physicalcpu` on
/// macOS, the hybrid core-type query on Windows and CPU capacity from sysfs on Linux.
#[derive(Clone, Copy, Default, Debug)]
pub struct StdPlatform;

impl Platform for StdPlatform {
    fn topology(&self) -> CoreTopology {
        let threads = std::thread::available_parallelism().map_or(2, |count| count.get() as u32);
        // Miri has none of the queries below.
        let detected = if cfg!(miri) { None } else { os::topology() };
        detected
            // No core information at all: assume two hardware threads per core.
            .unwrap_or_else(|| CoreTopology::homogeneous(threads.div_ceil(2)))
            // `available_parallelism` honours affinity masks and container limits.
            .limited_to(threads)
    }

    fn hardware_threads(&self) -> Option<u32> {
        // Honours affinity masks and container limits.
        Some(std::thread::available_parallelism().map_or(2, |count| count.get() as u32))
    }

    fn spawn_thread(
        &self,
        spec: ThreadSpec,
        body: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<ThreadHandle> {
        let class = spec.class;
        let thread = std::thread::Builder::new().name(spec.name).spawn(move || {
            // Miri cannot call the scheduling APIs, and has nothing to schedule.
            if !cfg!(miri)
                && let Err(error) = class.enter()
            {
                // The class is best effort; the thread runs correctly at default priority.
                tracing::warn!(target: TARGET, %error, ?class, "thread class not applied");
            }
            body();
        })?;
        Ok(ThreadHandle::new(move || {
            // A panic on the thread was already reported by the panic hook, and engine
            // threads abort on panic anyway; there is nothing to add here.
            let _ = thread.join();
        }))
    }
}

#[cfg(target_vendor = "apple")]
mod os {
    use std::collections::BTreeMap;

    use super::detect::apple_cpu_classes;
    use super::{CoreClass, CoreTopology};

    /// Reads an integer sysctl.
    pub(super) fn sysctl(name: &core::ffi::CStr) -> Option<u32> {
        let mut value: i32 = 0;
        let mut size = size_of::<i32>();
        // SAFETY: `name` is NUL-terminated, `value` is a live `i32` and `size` holds its
        // size, which is the contract of `sysctlbyname` for an integer value; no new value
        // is set.
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&raw mut value).cast(),
                &raw mut size,
                core::ptr::null_mut(),
                0,
            )
        };
        (status == 0 && size == size_of::<i32>()).then(|| u32::try_from(value).ok()).flatten()
    }

    pub(super) fn topology() -> Option<CoreTopology> {
        // Performance level 0 is the fastest class; Apple Silicon has two levels.
        let levels = sysctl(c"hw.nperflevels").unwrap_or(1);
        let performance = sysctl(c"hw.perflevel0.physicalcpu");
        match (levels, performance) {
            (2.., Some(performance)) if performance > 0 => Some(CoreTopology {
                performance,
                efficiency: (1..levels)
                    .filter_map(|level| {
                        let name = format!("hw.perflevel{level}.physicalcpu\0");
                        sysctl(core::ffi::CStr::from_bytes_with_nul(name.as_bytes()).ok()?)
                    })
                    .sum(),
            }),
            _ => sysctl(c"hw.physicalcpu").map(CoreTopology::homogeneous),
        }
    }

    /// The class of each logical CPU from the logical CPUs of each performance level; one
    /// level, the performance class, where the kernel reports no levels, as `topology`
    /// does. The counts are the boot's (`logicalcpu_max`), not the current power mode's
    /// (`logicalcpu`): a CPU's number is fixed in `[0, hw.ncpu)`, and a CPU taken offline
    /// would otherwise shift every number above it.
    pub(super) fn cpu_classes() -> BTreeMap<u32, CoreClass> {
        let levels = sysctl(c"hw.nperflevels").unwrap_or(1);
        let counts: Option<Vec<u32>> = if levels >= 2 {
            (0..levels)
                .map(|level| {
                    let name = format!("hw.perflevel{level}.logicalcpu_max\0");
                    sysctl(core::ffi::CStr::from_bytes_with_nul(name.as_bytes()).ok()?)
                })
                .collect()
        } else {
            sysctl(c"hw.logicalcpu_max").map(|count| vec![count])
        };
        counts.map_or_else(BTreeMap::new, |counts| apple_cpu_classes(&counts))
    }

    /// The number of the CPU the calling thread is on.
    pub(super) fn current_cpu() -> Option<u32> {
        let mut cpu: libc::size_t = 0;
        // SAFETY: `cpu` is a live `size_t`, the one out-pointer the call writes through.
        let status = unsafe { libc::pthread_cpu_number_np(&raw mut cpu) };
        (status == 0).then(|| u32::try_from(cpu).ok()).flatten()
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod os {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::detect::{linux_cpu_classes, linux_cpus, linux_topology};
    use super::{CoreClass, CoreTopology};

    /// The directory sysfs describes each logical CPU in.
    const CPU_ROOT: &str = "/sys/devices/system/cpu";

    /// The CPU list of a PMU, which Intel hybrid parts register for each core type.
    fn pmu_cpus(pmu: &str) -> Option<String> {
        std::fs::read_to_string(format!("/sys/devices/{pmu}/cpus")).ok()
    }

    pub(super) fn topology() -> Option<CoreTopology> {
        let cpus = linux_cpus(Path::new(CPU_ROOT))?;
        linux_topology(&cpus, pmu_cpus("cpu_core").as_deref(), pmu_cpus("cpu_atom").as_deref())
    }

    pub(super) fn cpu_classes() -> BTreeMap<u32, CoreClass> {
        let Some(cpus) = linux_cpus(Path::new(CPU_ROOT)) else { return BTreeMap::new() };
        linux_cpu_classes(&cpus, pmu_cpus("cpu_core").as_deref(), pmu_cpus("cpu_atom").as_deref())
    }

    /// The number of the CPU the calling thread is on.
    pub(super) fn current_cpu() -> Option<u32> {
        // SAFETY: `sched_getcpu` takes nothing and returns the CPU or -1.
        u32::try_from(unsafe { libc::sched_getcpu() }).ok()
    }
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
    };

    use std::collections::BTreeMap;

    use windows_sys::Win32::System::Kernel::PROCESSOR_NUMBER;
    use windows_sys::Win32::System::Threading::GetCurrentProcessorNumberEx;

    use super::detect::{
        from_ratings, windows_core_ratings, windows_processor, windows_processor_classes,
    };
    use super::{CoreClass, CoreTopology};

    pub(super) fn topology() -> Option<CoreTopology> {
        let ratings = windows_core_ratings(&core_records()?);
        (!ratings.is_empty()).then(|| from_ratings(&ratings))
    }

    pub(super) fn cpu_classes() -> BTreeMap<u32, CoreClass> {
        core_records().map_or_else(BTreeMap::new, |records| windows_processor_classes(&records))
    }

    /// The key `detect::windows_processor` gives the processor the calling thread is on.
    pub(super) fn current_cpu() -> Option<u32> {
        let mut number = PROCESSOR_NUMBER::default();
        // SAFETY: `number` is a live `PROCESSOR_NUMBER`, the one out-pointer the call writes
        // through; the call cannot fail.
        unsafe { GetCurrentProcessorNumberEx(&raw mut number) };
        Some(windows_processor(number.Group, number.Number))
    }

    /// The processor-core records of `GetLogicalProcessorInformationEx`, as plain bytes.
    fn core_records() -> Option<Vec<u8>> {
        let mut length = 0u32;
        // SAFETY: a null buffer with a zero length is the documented way to ask for the
        // required size, which is written to `length`.
        unsafe {
            GetLogicalProcessorInformationEx(
                RelationProcessorCore,
                core::ptr::null_mut(),
                &raw mut length,
            );
        }
        if length == 0 {
            return None;
        }
        // `u64` elements give the buffer the alignment the system's records expect.
        let mut buffer = vec![0u64; (length as usize).div_ceil(size_of::<u64>())];
        // SAFETY: `buffer` is at least `length` bytes long, writable, and aligned for the
        // record type; the system writes at most `length` bytes of records into it.
        let ok = unsafe {
            GetLogicalProcessorInformationEx(
                RelationProcessorCore,
                buffer.as_mut_ptr().cast(),
                &raw mut length,
            )
        };
        if ok == 0 {
            return None;
        }
        // The records are walked as plain bytes; nothing is read through a record type.
        Some(buffer.iter().flat_map(|word| word.to_ne_bytes()).take(length as usize).collect())
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android", windows)))]
mod os {
    use std::collections::BTreeMap;

    use super::{CoreClass, CoreTopology};

    pub(super) fn topology() -> Option<CoreTopology> {
        None
    }

    pub(super) fn cpu_classes() -> BTreeMap<u32, CoreClass> {
        BTreeMap::new()
    }

    pub(super) fn current_cpu() -> Option<u32> {
        None
    }
}

/// The class of each logical CPU, keyed by what [`current_cpu`] returns for it, as the
/// system reports the machine: empty where it reports nothing of the cores' types. For
/// measurement only (`crate::measure`).
pub(crate) fn cpu_classes() -> std::collections::BTreeMap<u32, CoreClass> {
    os::cpu_classes()
}

/// The CPU the calling thread is on at the call, as the key of [`cpu_classes`]. For
/// measurement only (`crate::measure`).
pub(crate) fn current_cpu() -> Option<u32> {
    os::current_cpu()
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use super::detect::{
        LinuxCpu, WindowsCore, apple_cpu_classes, from_ratings, linux_cpu_classes, linux_topology,
        parse_cpu_list, windows_core_ratings, windows_cores, windows_processor,
        windows_processor_classes,
    };

    fn topology(performance: u32, efficiency: u32) -> CoreTopology {
        CoreTopology { performance, efficiency }
    }

    #[test]
    fn ratings_classify_hybrid_and_homogeneous_cpus() {
        // Eight performance cores and sixteen efficiency cores.
        let mut hybrid = vec![1u32; 16];
        hybrid.extend([2; 8]);
        assert_eq!(from_ratings(&hybrid), topology(8, 16));
        // Three classes: everything above the slowest is frame-critical.
        assert_eq!(from_ratings(&[250, 250, 250, 250, 620, 620, 620, 1024]), topology(4, 4));
        // No core-type information: physical cores minus one, and no efficiency cores.
        assert_eq!(from_ratings(&[0; 8]), topology(7, 0));
        assert_eq!(from_ratings(&[1024]), topology(1, 0));
        assert_eq!(from_ratings(&[]), topology(1, 0));
    }

    #[test]
    fn worker_counts_are_capped_by_the_usable_hardware_threads() {
        // (hardware threads, detected topology) -> (frame-critical, background) workers.
        let counts = |threads: u32, detected: CoreTopology| {
            let limited = detected.limited_to(threads);
            let frame = default_frame_workers(limited, Some(threads));
            (frame, default_background_workers(limited, Some(threads), frame))
        };
        // Homogeneous, one hardware thread per core: all cores but one are frame-critical,
        // and the two low-priority background threads shrink to what is left.
        let homogeneous = CoreTopology::homogeneous;
        assert_eq!(counts(1, homogeneous(1)), (1, 1));
        assert_eq!(counts(2, homogeneous(2)), (1, 1));
        assert_eq!(counts(4, homogeneous(4)), (3, 1));
        assert_eq!(counts(8, homogeneous(8)), (7, 1));
        // Homogeneous with two hardware threads per core: room for both background threads.
        assert_eq!(counts(8, homogeneous(4)), (3, 2));
        assert_eq!(counts(16, homogeneous(8)), (7, 2));
        // Hybrid, half and half.
        assert_eq!(counts(2, topology(1, 1)), (1, 1));
        assert_eq!(counts(4, topology(2, 2)), (2, 2));
        assert_eq!(counts(8, topology(4, 4)), (4, 4));
        // A big machine seen through a small affinity mask or container limit: never more
        // workers than hardware threads, except for the minimum of one of each class.
        assert_eq!(counts(4, homogeneous(16)), (3, 1));
        assert_eq!(counts(4, topology(8, 16)), (3, 1));
        assert_eq!(counts(8, topology(8, 16)), (7, 1));
        assert_eq!(counts(12, topology(8, 16)), (8, 4));
        assert_eq!(counts(1, topology(8, 16)), (1, 1));
        for threads in [1, 2, 4, 8] {
            for detected in [homogeneous(threads), topology(threads.div_ceil(2), threads / 2)] {
                let (frame, background) = counts(threads, detected);
                assert!(frame + background <= threads.max(2), "{threads}: {detected:?}");
            }
        }
        // A platform that reports no thread count is taken at its word.
        assert_eq!(default_frame_workers(topology(8, 16), None), 8);
        assert_eq!(default_background_workers(topology(8, 0), None, 8), 2);
        assert_eq!(default_background_workers(topology(8, 16), None, 8), 16);
    }

    /// One record as Windows lays it out: `Relationship`, `Size`, then the payload.
    fn record(relationship: i32, size: u32, efficiency_class: u8) -> Vec<u8> {
        let mut bytes = vec![0u8; size.max(8) as usize];
        bytes[..4].copy_from_slice(&relationship.to_ne_bytes());
        bytes[4..8].copy_from_slice(&size.to_ne_bytes());
        if let Some(class) = bytes.get_mut(9) {
            *class = efficiency_class;
        }
        bytes
    }

    #[test]
    fn windows_records_are_walked_by_their_own_size() {
        // A processor-core record with one group is 48 bytes.
        let core = |class| record(0, 48, class);

        // Eight identical cores: every record counts, including the last, which is shorter
        // than the declared size of the record type.
        let homogeneous: Vec<u8> = (0..8).flat_map(|_| core(0)).collect();
        assert_eq!(windows_core_ratings(&homogeneous), [0; 8]);
        assert_eq!(from_ratings(&windows_core_ratings(&homogeneous)), topology(7, 0));

        // Six performance cores (class 1) and eight efficiency cores (class 0), with a
        // longer record of another kind in between, which is skipped by its size.
        let mut hybrid: Vec<u8> = (0..6).flat_map(|_| core(1)).collect();
        hybrid.extend(record(3, 76, 9));
        hybrid.extend((0..8).flat_map(|_| core(0)));
        let ratings = windows_core_ratings(&hybrid);
        assert_eq!(ratings.len(), 14);
        assert_eq!(from_ratings(&ratings), topology(6, 8));

        // A buffer cut off inside the last record: the whole records before it count.
        let truncated = &homogeneous[..homogeneous.len() - 10];
        assert_eq!(windows_core_ratings(truncated).len(), 7);
        // Cut off inside a header.
        assert_eq!(windows_core_ratings(&homogeneous[..48 + 5]).len(), 1);

        // A zero size would never advance; the walk stops there.
        let mut stuck = core(1);
        stuck.extend(record(0, 0, 1));
        stuck.extend(core(1));
        assert_eq!(windows_core_ratings(&stuck), [1]);
        // So does a size smaller than the header.
        let mut short = core(1);
        short.extend(record(0, 4, 1));
        assert_eq!(windows_core_ratings(&short), [1]);
        assert!(windows_core_ratings(&[]).is_empty());
    }

    /// A processor-core record with one group mask per `(group, mask)`, as Windows lays it
    /// out: the group count at offset 30, then a 16-byte `GROUP_AFFINITY` for each from
    /// offset 32.
    fn core_record(efficiency_class: u8, masks: &[(u16, u64)]) -> Vec<u8> {
        let mut bytes = record(0, 32 + 16 * masks.len() as u32, efficiency_class);
        bytes[30..32].copy_from_slice(&(masks.len() as u16).to_ne_bytes());
        for (index, &(group, mask)) in masks.iter().enumerate() {
            let start = 32 + 16 * index;
            bytes[start..start + 8].copy_from_slice(&mask.to_ne_bytes());
            bytes[start + 8..start + 10].copy_from_slice(&group.to_ne_bytes());
        }
        bytes
    }

    #[test]
    fn windows_records_give_each_core_its_processors() {
        use CoreClass::{Efficiency, Performance};

        // Two performance cores of two threads each, then two efficiency cores, in group
        // 0; a third performance core in group 1, at its processors 62 and 63.
        let mut records = core_record(1, &[(0, 0b11)]);
        records.extend(core_record(1, &[(0, 0b1100)]));
        records.extend(record(3, 76, 9));
        records.extend(core_record(0, &[(0, 1 << 4)]));
        records.extend(core_record(0, &[(0, 1 << 5)]));
        records.extend(core_record(1, &[(1, 0b11 << 62)]));
        let cores = windows_cores(&records);
        assert_eq!(cores.len(), 5);
        assert_eq!(cores[4].processors, [64 + 62, 64 + 63]);
        assert_eq!(windows_processor(1, 62), 64 + 62);
        assert_eq!(windows_core_ratings(&records), [1, 1, 0, 0, 1]);
        let mut expected = spans(&[(0, 3, Performance), (4, 5, Efficiency)]);
        expected.extend([(126, Performance), (127, Performance)]);
        assert_eq!(windows_processor_classes(&records), expected);

        // All of one class: every processor is a performance processor.
        let homogeneous: Vec<u8> =
            (0..4).flat_map(|core| core_record(0, &[(0, 1 << core)])).collect();
        assert_eq!(windows_processor_classes(&homogeneous), spans(&[(0, 3, Performance)]));

        // A core record of two group masks names the processors of both.
        let spread = core_record(0, &[(0, 1), (2, 1)]);
        assert_eq!(windows_cores(&spread)[0].processors, [0, 128]);
        // A record too short for the masks it counts keeps its class and the masks that
        // fit: the 48-byte records of the walk test count, with no processors.
        let mut short = core_record(1, &[(0, 1), (0, 2)]);
        short.truncate(48);
        short[4..8].copy_from_slice(&48u32.to_ne_bytes());
        assert_eq!(windows_cores(&short), [WindowsCore { rating: 1, processors: vec![0] }]);
        assert_eq!(windows_cores(&record(0, 48, 1))[0].processors, [0u32; 0]);
    }

    #[test]
    fn cpu_lists_parse() {
        assert_eq!(parse_cpu_list("0-3,8,10-11\n"), [0, 1, 2, 3, 8, 10, 11]);
        assert_eq!(parse_cpu_list("5"), [5]);
        assert_eq!(parse_cpu_list(""), [0u32; 0]);
        assert_eq!(parse_cpu_list("1,x,3-y,4"), [1, 4]);
    }

    /// `count` logical CPUs starting at `first`, `threads` per core.
    fn cpus(first: u32, count: u32, threads: u32, capacity: Option<u32>) -> Vec<LinuxCpu> {
        (first..first + count)
            .map(|cpu| LinuxCpu {
                cpu,
                package: 0,
                cluster: 0,
                core: cpu / threads * threads,
                capacity,
            })
            .collect()
    }

    #[test]
    fn linux_topology_from_capacity_pmu_lists_or_neither() {
        // arm64 big.LITTLE, three classes: 4 little, 3 big, 1 prime.
        let mut arm = cpus(0, 4, 1, Some(250));
        arm.extend(cpus(4, 3, 1, Some(620)));
        arm.extend(cpus(7, 1, 1, Some(1024)));
        assert_eq!(linux_topology(&arm, None, None), Some(topology(4, 4)));

        // Intel hybrid: 8 performance cores with two threads each (CPUs 0-15), 8
        // efficiency cores (16-23), no capacity files.
        let mut intel = cpus(0, 16, 2, None);
        intel.extend(cpus(16, 8, 1, None));
        assert_eq!(linux_topology(&intel, Some("0-15\n"), Some("16-23\n")), Some(topology(8, 8)));
        // The same machine on a kernel without the hybrid PMUs: 16 cores, no types.
        assert_eq!(linux_topology(&intel, None, None), Some(topology(15, 0)));

        // A homogeneous desktop with two threads per core, and one whose kernel exports
        // equal capacities.
        assert_eq!(linux_topology(&cpus(0, 16, 2, None), None, None), Some(topology(7, 0)));
        assert_eq!(linux_topology(&cpus(0, 4, 1, Some(1024)), None, None), Some(topology(3, 0)));
        // Two sockets: the same core id in different packages is two cores.
        let mut sockets = cpus(0, 4, 1, None);
        sockets.extend(cpus(0, 4, 1, None).into_iter().map(|cpu| LinuxCpu { package: 1, ..cpu }));
        assert_eq!(linux_topology(&sockets, None, None), Some(topology(7, 0)));
        assert_eq!(linux_topology(&[], None, None), None);
    }

    /// `(first, last, class)` spans as a map of each CPU's class.
    fn spans(spans: &[(u32, u32, CoreClass)]) -> BTreeMap<u32, CoreClass> {
        spans
            .iter()
            .flat_map(|&(first, last, class)| (first..=last).map(move |cpu| (cpu, class)))
            .collect()
    }

    #[test]
    fn linux_cpu_classes_follow_the_topologys_ratings() {
        use CoreClass::{Efficiency, Performance};

        // Three classes: the little cores alone are the efficiency class.
        let mut arm = cpus(0, 4, 1, Some(250));
        arm.extend(cpus(4, 3, 1, Some(620)));
        arm.extend(cpus(7, 1, 1, Some(1024)));
        assert_eq!(
            linux_cpu_classes(&arm, None, None),
            spans(&[(0, 3, Efficiency), (4, 7, Performance)])
        );
        // Intel hybrid: both threads of a performance core are performance CPUs.
        let mut intel = cpus(0, 16, 2, None);
        intel.extend(cpus(16, 8, 1, None));
        assert_eq!(
            linux_cpu_classes(&intel, Some("0-15\n"), Some("16-23\n")),
            spans(&[(0, 15, Performance), (16, 23, Efficiency)])
        );
        // A thread of a core takes the core's rating, the best of its threads'.
        let mut mixed = cpus(0, 2, 2, Some(512));
        mixed[1].capacity = Some(1024);
        mixed.extend(cpus(2, 2, 1, Some(512)));
        assert_eq!(
            linux_cpu_classes(&mixed, None, None),
            spans(&[(0, 1, Performance), (2, 3, Efficiency)])
        );
        // Equal capacities: one class, reported. No capacities and no PMU lists, or one
        // list alone: nothing is reported, where the topology still counts cores.
        assert_eq!(
            linux_cpu_classes(&cpus(0, 4, 1, Some(1024)), None, None),
            spans(&[(0, 3, Performance)])
        );
        assert!(linux_cpu_classes(&intel, None, None).is_empty());
        assert!(linux_cpu_classes(&intel, Some("0-15"), None).is_empty());
        assert!(linux_cpu_classes(&[], None, None).is_empty());
    }

    #[test]
    fn linux_device_tree_clusters_restart_core_ids() {
        use CoreClass::{Efficiency, Performance};

        // Device-tree arm64 (drivers/base/arch_topology.c): every CPU in package 0, and
        // `core_id` restarting at 0 in each cluster. Four little cores in cluster 0, four
        // big ones in cluster 1.
        let mut cpus: Vec<LinuxCpu> = (0..4)
            .map(|cpu| LinuxCpu { cpu, package: 0, cluster: 0, core: cpu, capacity: Some(446) })
            .collect();
        cpus.extend((4..8).map(|cpu| LinuxCpu {
            cpu,
            package: 0,
            cluster: 1,
            core: cpu - 4,
            capacity: Some(1024),
        }));
        assert_eq!(
            linux_cpu_classes(&cpus, None, None),
            spans(&[(0, 3, Efficiency), (4, 7, Performance)])
        );
        assert_eq!(linux_topology(&cpus, None, None), Some(topology(4, 4)));
    }

    /// A temporary directory, removed when dropped, so that a test that fails leaves none.
    struct TempTree(std::path::PathBuf);

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Writes a sysfs CPU tree under a fresh temporary directory: each CPU's files as
    /// `(cpu, [(relative path, contents)])`, plus the entries of the real directory that are
    /// not CPUs.
    fn sysfs_tree(name: &str, cpus: &[(u32, Vec<(&str, &str)>)]) -> TempTree {
        let tree = TempTree(
            std::env::temp_dir().join(format!("lactose-task-sysfs-{name}-{}", std::process::id())),
        );
        let root = &tree.0;
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root.join("cpuidle")).unwrap();
        std::fs::create_dir_all(root.join("cpufreq")).unwrap();
        std::fs::write(root.join("online"), "0-7\n").unwrap();
        for (cpu, files) in cpus {
            let dir = root.join(format!("cpu{cpu}"));
            std::fs::create_dir_all(&dir).unwrap();
            for (path, contents) in files {
                let path = dir.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, contents).unwrap();
            }
        }
        tree
    }

    #[test]
    #[cfg_attr(miri, ignore = "writes a directory tree")]
    fn linux_sysfs_is_read_with_and_without_cluster_ids() {
        use CoreClass::{Efficiency, Performance};

        let read = |tree: TempTree| {
            let mut cpus = super::detect::linux_cpus(&tree.0).unwrap();
            cpus.sort_by_key(|cpu| cpu.cpu);
            cpus
        };
        let cpu = |cpu, package, cluster, core, capacity| LinuxCpu {
            cpu,
            package,
            cluster,
            core,
            capacity,
        };

        // A current device-tree arm64 kernel: one package, core ids restarting in each
        // cluster; an offline CPU with no topology; a CPU whose cluster the kernel does not
        // know (-1) and that exports no capacity.
        let values: Vec<String> = (0..8).map(|cpu: u32| (cpu % 4).to_string()).collect();
        let mut tree: Vec<(u32, Vec<(&str, &str)>)> = (0..8)
            .map(|cpu: u32| {
                let little = cpu < 4;
                (
                    cpu,
                    vec![
                        ("topology/physical_package_id", "0\n"),
                        ("topology/core_id", values[cpu as usize].as_str()),
                        ("topology/cluster_id", if little { "0\n" } else { "1\n" }),
                        ("cpu_capacity", if little { "446\n" } else { "1024\n" }),
                    ],
                )
            })
            .collect();
        tree.push((8, vec![("cpu_capacity", "1024\n")]));
        tree.push((
            9,
            vec![
                ("topology/physical_package_id", "1"),
                ("topology/core_id", "0"),
                ("topology/cluster_id", "-1"),
            ],
        ));
        let cpus = read(sysfs_tree("clusters", &tree));
        let mut expected: Vec<LinuxCpu> = (0..4).map(|n| cpu(n, 0, 0, n, Some(446))).collect();
        expected.extend((4..8).map(|n| cpu(n, 0, 1, n - 4, Some(1024))));
        expected.push(cpu(9, 1, 0, 0, None));
        assert_eq!(cpus, expected);
        let classes = linux_cpu_classes(&cpus[..8], None, None);
        assert_eq!(classes, spans(&[(0, 3, Efficiency), (4, 7, Performance)]));

        // A kernel before 5.16: no cluster ids, every core its own id within the package.
        let tree: Vec<(u32, Vec<(&str, &str)>)> = (0..4)
            .map(|cpu: u32| {
                (
                    cpu,
                    vec![
                        ("topology/physical_package_id", "0"),
                        ("topology/core_id", values[cpu as usize].as_str()),
                    ],
                )
            })
            .collect();
        let cpus = read(sysfs_tree("no-clusters", &tree));
        assert_eq!(cpus, (0..4).map(|n| cpu(n, 0, 0, n, None)).collect::<Vec<_>>());
        assert!(super::detect::linux_cpus(std::path::Path::new("/nonexistent-sysfs")).is_none());
    }

    #[test]
    fn apple_cpu_classes_number_the_slowest_level_first() {
        use CoreClass::{Efficiency, Performance};

        // The M1: four performance cores at level 0, four efficiency cores at level 1.
        assert_eq!(apple_cpu_classes(&[4, 4]), spans(&[(0, 3, Efficiency), (4, 7, Performance)]));
        // The M1 Pro: eight and two.
        assert_eq!(apple_cpu_classes(&[8, 2]), spans(&[(0, 1, Efficiency), (2, 9, Performance)]));
        // Every level below level 0 is the efficiency class, as the topology counts them.
        assert_eq!(
            apple_cpu_classes(&[2, 3, 1]),
            spans(&[(0, 3, Efficiency), (4, 5, Performance)])
        );
        // One level: one class.
        assert_eq!(apple_cpu_classes(&[6]), spans(&[(0, 5, Performance)]));
        assert!(apple_cpu_classes(&[]).is_empty());
    }

    #[test]
    #[cfg_attr(miri, ignore = "queries the operating system")]
    fn detected_topology_is_usable() {
        let topology = StdPlatform.topology();
        // CI's macOS leg shows this line, so its log records what the runner reports.
        eprintln!("{topology:?}, classes {:?}", cpu_classes());
        #[cfg(target_vendor = "apple")]
        eprintln!("hw.nperflevels {:?}", super::os::sysctl(c"hw.nperflevels"));
        assert!(topology.performance >= 1);
        // Apple Silicon has efficiency cores, but a virtual machine (the GitHub runner) may
        // present one performance level; the assertion follows what the kernel reports.
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        if super::os::sysctl(c"hw.nperflevels").is_some_and(|levels| levels >= 2) {
            assert!(topology.efficiency >= 1, "two performance levels but no efficiency core");
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "applies an operating-system scheduling class")]
    fn spawned_threads_run_named_with_their_class_applied() {
        let spec = ThreadSpec {
            name: "whisk-test".to_owned(),
            class: ThreadClass::Background,
            index_in_class: 0,
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let body = Box::new(move || {
            let name = std::thread::current().name().map(str::to_owned);
            sender.send((name, lactose_core::thread_class::fp::is_deterministic())).unwrap();
        });
        StdPlatform.spawn_thread(spec, body).unwrap().join();
        assert_eq!(receiver.recv().unwrap(), (Some("whisk-test".to_owned()), true));
    }
}
