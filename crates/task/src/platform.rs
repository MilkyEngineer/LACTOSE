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

impl CoreTopology {
    /// The topology of a CPU that reports no core types: every physical core but one.
    fn homogeneous(physical_cores: u32) -> Self {
        Self { performance: physical_cores.saturating_sub(1).max(1), efficiency: 0 }
    }

    /// Caps the counts at the hardware threads this process may actually use, which an
    /// affinity mask or a container limit can put far below what the machine has. One
    /// thread is left for the platform thread, so performance cores are at most one fewer.
    fn limited_to(self, threads: u32) -> Self {
        let performance = default_frame_workers(self, Some(threads));
        Self { performance, efficiency: self.efficiency.min(threads.saturating_sub(performance)) }
    }
}

/// The default number of frame-critical workers: the performance cores, but at most one
/// fewer than the hardware threads the process may use, so the platform thread has
/// somewhere to run, and at least one.
pub(crate) fn default_frame_workers(topology: CoreTopology, threads: Option<u32>) -> u32 {
    let cap = threads.map_or(u32::MAX, |threads| threads.saturating_sub(1));
    topology.performance.min(cap).max(1)
}

/// The default number of background workers beside `frame` frame-critical ones: the
/// efficiency cores, or two on a homogeneous CPU, but no more than the hardware threads the
/// frame-critical workers leave, and at least one.
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

    use super::CoreTopology;

    /// Classifies physical cores by a per-core rating where a higher value means a faster
    /// core (Linux CPU capacity, the Windows efficiency class). The slowest class is the
    /// efficiency cores and every class above it is frame-critical, which on a three-class
    /// part counts the middle cores with the fastest. Ratings that are all equal carry no
    /// core-type information.
    pub(super) fn from_ratings(ratings: &[u32]) -> CoreTopology {
        let cores = ratings.len() as u32;
        let Some(&slowest) = ratings.iter().min() else { return CoreTopology::homogeneous(1) };
        let efficiency = ratings.iter().filter(|&&rating| rating == slowest).count() as u32;
        if efficiency == cores {
            CoreTopology::homogeneous(cores)
        } else {
            CoreTopology { performance: cores - efficiency, efficiency }
        }
    }

    /// The efficiency class of every physical core in a buffer filled by
    /// `GetLogicalProcessorInformationEx(RelationProcessorCore)`.
    ///
    /// The buffer is a sequence of variable-length records. Each starts with an eight-byte
    /// header, `Relationship: i32` then `Size: u32`, and a processor-core record carries
    /// its efficiency class in the tenth byte. A core record is 48 bytes on a machine with
    /// one processor group, shorter than the declared size of the record type, which is
    /// sized for its largest variant; only the header and the record's own `Size` say how
    /// far a record reaches. A record that would run past the buffer, or that claims a
    /// size smaller than its header, ends the walk.
    pub(super) fn windows_core_ratings(records: &[u8]) -> Vec<u32> {
        const HEADER: usize = 8;
        const RELATION_PROCESSOR_CORE: i32 = 0;
        const EFFICIENCY_CLASS: usize = 9;

        let mut ratings = Vec::new();
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
                ratings.push(u32::from(class));
            }
            offset += size;
        }
        ratings
    }

    /// One logical CPU as Linux sysfs describes it.
    #[derive(Clone, Copy, Debug)]
    pub(super) struct LinuxCpu {
        pub(super) cpu: u32,
        /// `topology/physical_package_id`.
        pub(super) package: u32,
        /// `topology/core_id`.
        pub(super) core: u32,
        /// `cpu_capacity`, where the kernel exports it (arm64, RISC-V).
        pub(super) capacity: Option<u32>,
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
        let has_capacity = cpus.iter().any(|cpu| cpu.capacity.is_some());
        let fast = performance_cpus.map(parse_cpu_list).unwrap_or_default();
        let slow = efficiency_cpus.map(parse_cpu_list).unwrap_or_default();
        let hybrid = !fast.is_empty() && !slow.is_empty();

        // One rating per physical core; hardware threads of a core share it.
        let mut cores = BTreeMap::new();
        for cpu in cpus {
            let rating = if has_capacity {
                cpu.capacity.unwrap_or(0)
            } else if hybrid {
                u32::from(fast.contains(&cpu.cpu))
            } else {
                0
            };
            let best = cores.entry((cpu.package, cpu.core)).or_insert(0);
            *best = rating.max(*best);
        }
        if cores.is_empty() {
            return None;
        }
        let ratings: Vec<u32> = cores.into_values().collect();
        Some(from_ratings(&ratings))
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
    use super::CoreTopology;

    /// Reads an integer sysctl.
    fn sysctl(name: &core::ffi::CStr) -> Option<u32> {
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
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod os {
    use std::path::Path;

    use super::CoreTopology;
    use super::detect::{LinuxCpu, linux_topology};

    fn read_u32(path: &Path) -> Option<u32> {
        std::fs::read_to_string(path).ok()?.trim().parse().ok()
    }

    pub(super) fn topology() -> Option<CoreTopology> {
        let mut cpus = Vec::new();
        for entry in std::fs::read_dir("/sys/devices/system/cpu").ok()?.flatten() {
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
            let capacity = read_u32(&path.join("cpu_capacity"));
            cpus.push(LinuxCpu { cpu, package, core, capacity });
        }
        let list = |pmu: &str| std::fs::read_to_string(format!("/sys/devices/{pmu}/cpus")).ok();
        linux_topology(&cpus, list("cpu_core").as_deref(), list("cpu_atom").as_deref())
    }
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
    };

    use super::CoreTopology;
    use super::detect::{from_ratings, windows_core_ratings};

    pub(super) fn topology() -> Option<CoreTopology> {
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
        let bytes: Vec<u8> =
            buffer.iter().flat_map(|word| word.to_ne_bytes()).take(length as usize).collect();
        let ratings = windows_core_ratings(&bytes);
        (!ratings.is_empty()).then(|| from_ratings(&ratings))
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android", windows)))]
mod os {
    use super::CoreTopology;

    pub(super) fn topology() -> Option<CoreTopology> {
        None
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;

    use super::detect::{
        LinuxCpu, from_ratings, linux_topology, parse_cpu_list, windows_core_ratings,
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
            .map(|cpu| LinuxCpu { cpu, package: 0, core: cpu / threads * threads, capacity })
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

    #[test]
    #[cfg_attr(miri, ignore = "queries the operating system")]
    fn detected_topology_is_usable() {
        let topology = StdPlatform.topology();
        assert!(topology.performance >= 1);
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert!(topology.efficiency >= 1, "Apple Silicon has efficiency cores");
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
