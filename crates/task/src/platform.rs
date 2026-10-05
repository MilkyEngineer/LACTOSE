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

    /// Classifies physical cores by a per-core rating where a higher value means a faster
    /// core (Linux CPU capacity, the Windows efficiency class). The fastest class is the
    /// performance cores. Ratings that are all equal carry no core-type information.
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "android", windows)),
        allow(dead_code, reason = "macOS reports its core classes directly")
    )]
    fn from_ratings(ratings: &[u32]) -> Self {
        let cores = ratings.len() as u32;
        let Some(&fastest) = ratings.iter().max() else { return Self::homogeneous(1) };
        let performance = ratings.iter().filter(|&&rating| rating == fastest).count() as u32;
        if performance == cores {
            Self::homogeneous(cores)
        } else {
            Self { performance, efficiency: cores - performance }
        }
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
        // Miri has none of the queries below.
        let detected = if cfg!(miri) { None } else { os::topology() };
        detected.unwrap_or_else(|| {
            // No core information at all: assume two hardware threads per core.
            let threads =
                std::thread::available_parallelism().map_or(2, |count| count.get() as u32);
            CoreTopology::homogeneous(threads.div_ceil(2))
        })
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
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::CoreTopology;

    fn read_u32(path: &Path) -> Option<u32> {
        std::fs::read_to_string(path).ok()?.trim().parse().ok()
    }

    /// One rating per physical core: its CPU capacity, or zero where the kernel exports
    /// none (x86 kernels without hybrid capacity scaling, most virtual machines).
    pub(super) fn topology() -> Option<CoreTopology> {
        let mut cores = BTreeMap::new();
        for entry in std::fs::read_dir("/sys/devices/system/cpu").ok()?.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let is_cpu = name.strip_prefix("cpu").is_some_and(|rest| {
                !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
            });
            if !is_cpu {
                continue;
            }
            let path = entry.path();
            // An offline CPU has no topology directory.
            let (Some(package), Some(core)) = (
                read_u32(&path.join("topology/physical_package_id")),
                read_u32(&path.join("topology/core_id")),
            ) else {
                continue;
            };
            let capacity = read_u32(&path.join("cpu_capacity")).unwrap_or(0);
            // Hardware threads of one core share a capacity; keep the largest seen.
            let rating = cores.entry((package, core)).or_insert(0);
            *rating = capacity.max(*rating);
        }
        if cores.is_empty() {
            return None;
        }
        let ratings: Vec<u32> = cores.into_values().collect();
        Some(CoreTopology::from_ratings(&ratings))
    }
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
        SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };

    use super::CoreTopology;

    /// One rating per physical core: its efficiency class, which is higher for faster
    /// cores on hybrid CPUs and zero everywhere on homogeneous ones.
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
        // `u64` elements give the buffer the alignment the records need.
        let mut buffer = vec![0u64; (length as usize).div_ceil(size_of::<u64>())];
        let base = buffer.as_mut_ptr().cast::<u8>();
        // SAFETY: `buffer` is at least `length` bytes long and suitably aligned.
        let ok = unsafe {
            GetLogicalProcessorInformationEx(RelationProcessorCore, base.cast(), &raw mut length)
        };
        if ok == 0 {
            return None;
        }

        let header = size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
        let mut ratings = Vec::new();
        let mut offset = 0usize;
        while offset + header <= length as usize {
            // SAFETY: the system wrote a sequence of records into the first `length` bytes
            // of `buffer`, each starting where the previous one's `Size` says and each at
            // least `header` bytes long, which the loop condition checks. `read_unaligned`
            // asks nothing of the record's alignment.
            let record = unsafe {
                base.add(offset).cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>().read_unaligned()
            };
            if record.Size == 0 {
                break;
            }
            if record.Relationship == RelationProcessorCore {
                // SAFETY: `Relationship` says the union holds a `PROCESSOR_RELATIONSHIP`.
                ratings.push(u32::from(unsafe { record.Anonymous.Processor.EfficiencyClass }));
            }
            offset += record.Size as usize;
        }
        (!ratings.is_empty()).then(|| CoreTopology::from_ratings(&ratings))
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

    #[test]
    fn ratings_classify_hybrid_and_homogeneous_cpus() {
        // Eight performance cores and sixteen efficiency cores.
        let mut hybrid = vec![1u32; 16];
        hybrid.extend([2; 8]);
        assert_eq!(
            CoreTopology::from_ratings(&hybrid),
            CoreTopology { performance: 8, efficiency: 16 }
        );
        // big.LITTLE with three classes: only the fastest is frame-critical.
        assert_eq!(
            CoreTopology::from_ratings(&[250, 250, 250, 250, 620, 620, 620, 1024]),
            CoreTopology { performance: 1, efficiency: 7 }
        );
        // No core-type information: physical cores minus one, and no efficiency cores.
        assert_eq!(
            CoreTopology::from_ratings(&[0; 8]),
            CoreTopology { performance: 7, efficiency: 0 }
        );
        assert_eq!(
            CoreTopology::from_ratings(&[1024]),
            CoreTopology { performance: 1, efficiency: 0 }
        );
        assert_eq!(CoreTopology::from_ratings(&[]), CoreTopology { performance: 1, efficiency: 0 });
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
