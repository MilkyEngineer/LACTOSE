//! Two calls for measurement: the task role a process runs in, and the class of the core a
//! thread is on. Cost tests and the benchmark call them, once per run and around the work
//! they time. The executor never calls them, and nothing in a frame should: the role is the
//! process's, and the class is a reading of where the scheduler put a thread, which the next
//! instruction may change.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use crate::platform::{self, CoreClass};

/// The class of the core the calling thread is on at the call, or `None` where it cannot
/// tell. For measurement: the thread may move to another core as soon as the call returns.
///
/// - macOS: the CPU number from `pthread_cpu_number_np`, read against the logical CPUs of
///   each performance level at boot (`hw.perflevel<n>.logicalcpu_max`). Level 0 is the
///   performance class and every other level the efficiency class, as [`CoreTopology`]
///   counts them.
///   macOS documents no map from a CPU number to its level: the call assumes that the
///   numbering runs from the slowest level up, efficiency cores first, as the device tree of
///   Apple silicon reports it (`README.md`, "Measurement calls").
/// - Windows: the processor number from `GetCurrentProcessorNumberEx`, read against the
///   efficiency class of its core, which `GetLogicalProcessorInformationEx` reports.
/// - Linux: the CPU from `sched_getcpu`, read against the CPU capacity of its core, or on an
///   Intel hybrid part the lists of the two core types' PMUs, as the topology reads them.
///   `None` where the kernel exports neither.
///
/// Where the system reports every core as one class, every core is
/// [`CoreClass::Performance`]. `None` on every other target, and under Miri. The machine's
/// classes are read once per process, at the first call; each call after that reads only
/// the current CPU.
///
/// [`CoreTopology`]: crate::CoreTopology
#[must_use]
pub fn current_core_class() -> Option<CoreClass> {
    static CLASSES: OnceLock<BTreeMap<u32, CoreClass>> = OnceLock::new();
    // Miri has none of the queries.
    if cfg!(miri) {
        return None;
    }
    let classes = CLASSES.get_or_init(platform::cpu_classes);
    classes.get(&platform::current_cpu()?).copied()
}

/// The task role of the process, as [`take_foreground_task_role`] found it, and what it
/// grants.
///
/// `priority` is the scheduled priority (`pth_priority` of `THREAD_EXTENDED_INFO`) of a
/// thread started after the request in [`ThreadClass::FrameCritical`], user-interactive QoS.
/// It is the record of what the run was granted, whatever it reads: it depends on how the
/// process was started, not only on the role, and the readings measured on the M1 are in
/// `README.md`, "Measurement calls" (47 from a terminal, 37 for a launchd job of
/// `ProcessType` `Interactive`, 20 or 4 for other launchd jobs and under `taskpolicy`).
/// `None` where the thread could not take its class or the query failed.
///
/// [`ThreadClass::FrameCritical`]: lactose_core::ThreadClass::FrameCritical
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskRole {
    /// The target has no task roles: every target but macOS. Nothing was asked.
    NotApplicable,
    /// The role macOS reports after the request.
    Reported {
        /// A `task_role_t` of `<mach/task_policy.h>`: 1, `TASK_FOREGROUND_APPLICATION`,
        /// where the request was accepted. This is the role as requested: a clamp on the
        /// process (`taskpolicy -b` or `-c`, a launchd background job) does not show in it.
        role: i32,
        /// The request's `kern_return_t`: 0 where it was accepted.
        request: i32,
        /// What the role grants, as [`TaskRole`] describes it.
        priority: Option<i32>,
    },
    /// macOS reported no role.
    Unreported {
        /// The request's `kern_return_t`.
        request: i32,
        /// The query's `kern_return_t`.
        query: i32,
        /// What the role grants, as [`TaskRole`] describes it.
        priority: Option<i32>,
    },
}

impl TaskRole {
    /// Whether the role reads back as the foreground-application role: whether the request
    /// was recorded, not what it granted, which `priority` shows.
    #[must_use]
    pub fn is_foreground(self) -> bool {
        matches!(self, Self::Reported { role: FOREGROUND_APPLICATION, .. })
    }
}

/// `TASK_FOREGROUND_APPLICATION` of `<mach/task_policy.h>`.
const FOREGROUND_APPLICATION: i32 = 1;

impl fmt::Display for TaskRole {
    /// The role's name in `<mach/task_policy.h>` and the two calls' results, or `not
    /// applicable`: the text a result file records.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotApplicable => formatter.write_str("not applicable"),
            Self::Reported { role, request, priority } => {
                let name = match role {
                    -1 => "TASK_RENICED",
                    0 => "TASK_UNSPECIFIED",
                    1 => "TASK_FOREGROUND_APPLICATION",
                    2 => "TASK_BACKGROUND_APPLICATION",
                    3 => "TASK_CONTROL_APPLICATION",
                    4 => "TASK_GRAPHICS_SERVER",
                    5 => "TASK_THROTTLE_APPLICATION",
                    6 => "TASK_NONUI_APPLICATION",
                    7 => "TASK_DEFAULT_APPLICATION",
                    8 => "TASK_DARWINBG_APPLICATION",
                    9 => "TASK_USER_INIT_APPLICATION",
                    _ => {
                        write!(formatter, "role {role} (set returned {request}, get returned 0)")?;
                        return write_priority(formatter, priority);
                    }
                };
                write!(formatter, "{name} (set returned {request}, get returned 0)")?;
                write_priority(formatter, priority)
            }
            Self::Unreported { request, query, priority } => {
                write!(formatter, "unknown (set returned {request}, get returned {query})")?;
                write_priority(formatter, priority)
            }
        }
    }
}

fn write_priority(formatter: &mut fmt::Formatter<'_>, priority: Option<i32>) -> fmt::Result {
    match priority {
        Some(priority) => write!(formatter, ", user-interactive priority {priority}"),
        None => formatter.write_str(", user-interactive priority unknown"),
    }
}

/// Asks for the task role of the frontmost application on macOS and returns the role the
/// system then reports, with the priority a user-interactive thread is then given. For
/// measurement: a cost test or a benchmark takes it once, before it times anything, and
/// records both.
///
/// A process started from a terminal is in `TASK_UNSPECIFIED`, where user-interactive QoS is
/// clamped to priority 31 (`docs/verification.md`, the task role). The role read back is the
/// request, which macOS accepts however the process was started; the priority is what was
/// granted, which the role raises from a terminal and not from every launchd job
/// ([`TaskRole`]). The role is the whole process's. It cannot be given back: macOS refuses a
/// request for `TASK_UNSPECIFIED`, so a case that needs that role runs in a process of its
/// own (`README.md`, "Measurement calls"). Every other target has no task roles and returns
/// [`TaskRole::NotApplicable`] without asking.
#[must_use = "the role the system reports is the record of the run"]
pub fn take_foreground_task_role() -> TaskRole {
    os::take_foreground_task_role()
}

#[cfg(target_os = "macos")]
mod os {
    use lactose_core::ThreadClass;

    use super::{FOREGROUND_APPLICATION, TaskRole};

    /// `TASK_CATEGORY_POLICY`, whose policy is one `integer_t`, the role.
    const CATEGORY_POLICY: u32 = 1;

    unsafe extern "C" {
        fn task_policy_set(
            task: libc::mach_port_t,
            flavor: u32,
            policy: *const i32,
            count: u32,
        ) -> libc::kern_return_t;
        fn task_policy_get(
            task: libc::mach_port_t,
            flavor: u32,
            policy: *mut i32,
            count: *mut u32,
            get_default: *mut u32,
        ) -> libc::kern_return_t;
    }

    #[allow(
        deprecated,
        reason = "libc deprecates mach_task_self in favour of a crate this one does not need"
    )]
    pub(super) fn take_foreground_task_role() -> TaskRole {
        let role = FOREGROUND_APPLICATION;
        let mut current = 0i32;
        let mut count = 1u32;
        // Zero asks for the current role rather than the default one.
        let mut get_default = 0u32;
        // SAFETY: both calls take the process's own task port and the category flavor,
        // whose policy is one `integer_t`: the request a pointer to one live `i32` with a
        // count of one, the query live out-pointers for the role, the count and the default
        // flag. They write only through those pointers.
        let (request, query) = unsafe {
            let task = libc::mach_task_self();
            let request = task_policy_set(task, CATEGORY_POLICY, &raw const role, 1);
            let query = task_policy_get(
                task,
                CATEGORY_POLICY,
                &raw mut current,
                &raw mut count,
                &raw mut get_default,
            );
            (request, query)
        };
        let priority = user_interactive_priority();
        if query == 0 && count == 1 {
            TaskRole::Reported { role: current, request, priority }
        } else {
            TaskRole::Unreported { request, query, priority }
        }
    }

    /// The scheduled priority of a new thread in the frame-critical class, which the
    /// process's role and any clamp on it decide.
    pub(super) fn user_interactive_priority() -> Option<i32> {
        std::thread::spawn(|| {
            ThreadClass::FrameCritical.enter().ok()?;
            let mut info = libc::thread_extended_info::default();
            let mut count = libc::THREAD_EXTENDED_INFO_COUNT;
            // SAFETY: `pthread_mach_thread_np` returns the calling thread's port without
            // adding a reference, live while the thread runs. `thread_info` writes at most
            // `count` integers through `info`, a live `thread_extended_info` of that size,
            // and the new count through `count`.
            let status = unsafe {
                libc::thread_info(
                    libc::pthread_mach_thread_np(libc::pthread_self()),
                    libc::THREAD_EXTENDED_INFO as libc::thread_flavor_t,
                    (&raw mut info).cast(),
                    &raw mut count,
                )
            };
            // A shorter answer than the extended one would leave the priority unwritten.
            (status == 0 && count == libc::THREAD_EXTENDED_INFO_COUNT).then_some(info.pth_priority)
        })
        .join()
        .ok()
        .flatten()
    }
}

#[cfg(not(target_os = "macos"))]
mod os {
    use super::TaskRole;

    pub(super) fn take_foreground_task_role() -> TaskRole {
        TaskRole::NotApplicable
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;

    #[test]
    fn a_role_is_recorded_by_its_name_and_the_calls_results() {
        let foreground = TaskRole::Reported { role: 1, request: 0, priority: Some(47) };
        assert!(foreground.is_foreground());
        assert_eq!(
            foreground.to_string(),
            "TASK_FOREGROUND_APPLICATION (set returned 0, get returned 0), user-interactive \
             priority 47"
        );
        // A clamped process reads back the role it asked for; the priority shows the clamp.
        let clamped = TaskRole::Reported { role: 1, request: 0, priority: Some(4) };
        assert!(clamped.is_foreground());
        assert!(clamped.to_string().ends_with("user-interactive priority 4"), "{clamped}");
        let refused = TaskRole::Reported { role: 0, request: 4, priority: Some(31) };
        assert!(!refused.is_foreground());
        assert_eq!(
            refused.to_string(),
            "TASK_UNSPECIFIED (set returned 4, get returned 0), user-interactive priority 31"
        );
        assert_eq!(
            TaskRole::Reported { role: 7, request: 0, priority: None }.to_string(),
            "TASK_DEFAULT_APPLICATION (set returned 0, get returned 0), user-interactive \
             priority unknown"
        );
        assert_eq!(
            TaskRole::Reported { role: 42, request: 0, priority: Some(46) }.to_string(),
            "role 42 (set returned 0, get returned 0), user-interactive priority 46"
        );
        let unreported = TaskRole::Unreported { request: 0, query: 5, priority: None };
        assert!(!unreported.is_foreground());
        assert_eq!(
            unreported.to_string(),
            "unknown (set returned 0, get returned 5), user-interactive priority unknown"
        );
        assert!(!TaskRole::NotApplicable.is_foreground());
        assert_eq!(TaskRole::NotApplicable.to_string(), "not applicable");
    }

    #[test]
    #[cfg_attr(miri, ignore = "asks the operating system for the task role")]
    fn the_foreground_role_is_taken_and_read_back() {
        #[cfg(target_os = "macos")]
        {
            // nextest runs each test in a process of its own, so this is the role the
            // process started in.
            let before = os::user_interactive_priority();
            let role = take_foreground_task_role();
            eprintln!("before the role: user-interactive priority {before:?}; after: {role}");
            let TaskRole::Reported { role: 1, request: 0, priority: Some(priority) } = role else {
                panic!("the foreground role was not read back with a priority: {role}");
            };
            // A clamp the process started under stays and is seen here, not in the role.
            assert!(Some(priority) >= before, "{role}, {before:?} before it");
            // From a terminal, `TASK_UNSPECIFIED` holds user-interactive QoS at 31; the role
            // lifts it to the class's own priority.
            if before == Some(31) {
                assert_eq!(priority, 47, "{role}");
            }
            // Taking it again changes nothing.
            assert_eq!(take_foreground_task_role(), role);
        }
        #[cfg(not(target_os = "macos"))]
        assert_eq!(take_foreground_task_role(), TaskRole::NotApplicable);
    }

    #[test]
    #[cfg_attr(miri, ignore = "queries the operating system")]
    fn the_class_of_the_current_core_is_read_on_every_thread() {
        let classes = platform::cpu_classes();
        let threads: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(|| (0..1_000).map(|_| current_core_class()).collect::<Vec<_>>())
            })
            .collect();
        for thread in threads {
            let seen = thread.join().unwrap();
            // Wherever the system reports the cores' types, every reading is a class.
            if !classes.is_empty() {
                assert!(seen.iter().all(Option::is_some), "{classes:?}");
            }
        }
        // Apple silicon reports its levels; a virtual machine may report one, and then every
        // core is a performance core.
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert!(!classes.is_empty(), "macOS reported no CPUs");
    }

    /// macOS cannot pin a thread, so the call is held to the current CPU by filling every
    /// core: as many spinning threads as CPUs, which the performance cores alone cannot
    /// hold, read both classes between them. A call that read any one CPU's class would see
    /// one. Each thread stops once both are seen, or after two seconds.
    #[test]
    #[cfg_attr(miri, ignore = "queries the operating system")]
    #[cfg(target_vendor = "apple")]
    fn every_core_busy_reads_both_classes() {
        use std::sync::atomic::{AtomicU8, Ordering};
        use std::time::{Duration, Instant};

        let classes = platform::cpu_classes();
        if !classes.values().any(|&class| class == CoreClass::Efficiency) {
            eprintln!("one performance level; nothing to tell apart");
            return;
        }
        // Bit 0: a performance reading; bit 1: an efficiency reading.
        let seen = std::sync::Arc::new(AtomicU8::new(0));
        let deadline = Instant::now() + Duration::from_secs(2);
        let threads: Vec<_> = (0..classes.len())
            .map(|_| {
                let seen = std::sync::Arc::clone(&seen);
                std::thread::spawn(move || {
                    lactose_core::ThreadClass::FrameCritical.enter().unwrap();
                    while seen.load(Ordering::Relaxed) != 0b11 && Instant::now() < deadline {
                        let bit = match current_core_class() {
                            Some(CoreClass::Performance) => 0b01,
                            Some(CoreClass::Efficiency) => 0b10,
                            None => panic!("a CPU the map does not hold"),
                        };
                        seen.fetch_or(bit, Ordering::Relaxed);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(seen.load(Ordering::Relaxed), 0b11, "{} CPUs busy", classes.len());
    }

    /// The map agrees with the topology the executor sizes its pools by, on a machine whose
    /// logical CPUs are its physical cores.
    #[test]
    #[cfg_attr(miri, ignore = "queries the operating system")]
    #[cfg(target_vendor = "apple")]
    fn the_classes_agree_with_the_topology() {
        let classes = platform::cpu_classes();
        let count = |class| classes.values().filter(|&&seen| seen == class).count() as u32;
        // Unlimited by an affinity mask, which macOS does not have.
        let topology = crate::Platform::topology(&crate::StdPlatform);
        if topology.efficiency == 0 {
            assert_eq!(count(CoreClass::Efficiency), 0);
        } else {
            assert_eq!(
                (count(CoreClass::Performance), count(CoreClass::Efficiency)),
                (topology.performance, topology.efficiency)
            );
        }
    }

    /// The numbering `current_core_class` assumes on macOS, held against the device tree,
    /// which names each CPU's cluster type by its logical CPU id.
    #[test]
    #[cfg_attr(miri, ignore = "runs ioreg")]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn the_apple_numbering_is_the_device_trees() {
        let output = std::process::Command::new("ioreg")
            .args(["-p", "IODeviceTree", "-n", "cpus", "-r", "-d", "2", "-l"])
            .output();
        let Some(output) = output.ok().filter(|output| output.status.success()) else {
            eprintln!("ioreg did not run; the numbering is not checked");
            return;
        };
        let tree = device_tree_classes(&String::from_utf8_lossy(&output.stdout));
        let classes = platform::cpu_classes();
        let levels = classes.values().any(|&class| class == CoreClass::Efficiency);
        if !levels {
            eprintln!("one performance level; the numbering is not checked");
            return;
        }
        // With two levels the tree must parse: a change in ioreg's format fails here rather
        // than turning the check off.
        assert!(!tree.is_empty(), "two performance levels but no cluster types parsed");
        assert_eq!(classes, tree);
    }

    /// Each CPU's class from the device tree's entries: a `logical-cpu-id` and a
    /// `cluster-type` of `E` or `P`, in any order within the entry.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn device_tree_classes(tree: &str) -> BTreeMap<u32, CoreClass> {
        let mut classes = BTreeMap::new();
        for entry in tree.split("+-o ").skip(1) {
            let value = |key: &str| {
                entry.lines().find_map(|line| {
                    line.trim_start_matches([' ', '|']).strip_prefix(&format!("\"{key}\" = "))
                })
            };
            let cpu = value("logical-cpu-id").and_then(|id| id.trim().parse().ok());
            let class = match value("cluster-type").map(str::trim) {
                Some("<\"E\">") => Some(CoreClass::Efficiency),
                Some("<\"P\">") => Some(CoreClass::Performance),
                _ => None,
            };
            if let (Some(cpu), Some(class)) = (cpu, class) {
                classes.insert(cpu, class);
            }
        }
        classes
    }

    #[test]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn device_tree_entries_parse() {
        let tree = r#"
  +-o cpu0@0  <class IOPlatformDevice>
  |     "logical-cpu-id" = 0
  |     "cluster-type" = <"E">
  +-o cpu1@100  <class IOPlatformDevice>
        "cluster-type" = <"P">
        "cpu-id" = <01000000>
        "logical-cpu-id" = 1
"#;
        assert_eq!(
            device_tree_classes(tree),
            BTreeMap::from([(0, CoreClass::Efficiency), (1, CoreClass::Performance)])
        );
    }

    /// Pins a thread to each CPU the process may use in turn: the class read there is the
    /// same at every call and is that CPU's.
    #[test]
    #[cfg_attr(miri, ignore = "pins a thread")]
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn the_class_is_stable_on_a_pinned_thread() {
        let classes = platform::cpu_classes();
        std::thread::spawn(move || {
            const SET: usize = size_of::<libc::cpu_set_t>();
            // SAFETY: a `cpu_set_t` is an array of integers, for which all zeros is valid.
            let mut allowed: libc::cpu_set_t = unsafe { std::mem::zeroed() };
            // SAFETY: `allowed` is a live `cpu_set_t` of `SET` bytes, the one out-pointer
            // the call writes through; pid 0 is the calling thread.
            let status = unsafe { libc::sched_getaffinity(0, SET, &raw mut allowed) };
            assert_eq!(status, 0, "sched_getaffinity");
            let cpus: Vec<usize> = (0..libc::CPU_SETSIZE as usize)
                // SAFETY: the macro reads the bit of `cpu`, below the set's size, of a live
                // set.
                .filter(|&cpu| unsafe { libc::CPU_ISSET(cpu, &allowed) })
                .collect();
            for cpu in cpus {
                // SAFETY: as for `allowed`: all zeros is a valid set, the macro sets one bit
                // below its size, and the call reads `SET` bytes of the live set.
                let status = unsafe {
                    let mut one: libc::cpu_set_t = std::mem::zeroed();
                    libc::CPU_SET(cpu, &mut one);
                    libc::sched_setaffinity(0, SET, &raw const one)
                };
                assert_eq!(status, 0, "sched_setaffinity to CPU {cpu}");
                let seen: Vec<_> = (0..100).map(|_| current_core_class()).collect();
                assert!(seen.iter().all(|&class| class == seen[0]), "CPU {cpu}: {seen:?}");
                assert_eq!(platform::current_cpu(), u32::try_from(cpu).ok());
                assert_eq!(
                    seen[0],
                    u32::try_from(cpu).ok().and_then(|cpu| classes.get(&cpu).copied())
                );
            }
        })
        .join()
        .unwrap();
    }

    /// As on Linux: pins a thread to each processor of its group the process may use.
    #[test]
    #[cfg_attr(miri, ignore = "pins a thread")]
    #[cfg(windows)]
    fn the_class_is_stable_on_a_pinned_thread() {
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, GetCurrentThread, GetProcessAffinityMask, SetThreadAffinityMask,
        };

        let classes = platform::cpu_classes();
        std::thread::spawn(move || {
            let (mut process, mut system) = (0usize, 0usize);
            // SAFETY: the pseudo handle of the current process is always valid, and the two
            // out-pointers are live `usize`s.
            let ok = unsafe {
                GetProcessAffinityMask(GetCurrentProcess(), &raw mut process, &raw mut system)
            };
            assert_ne!(ok, 0, "GetProcessAffinityMask");
            for bit in (0..usize::BITS).filter(|bit| process & (1 << bit) != 0) {
                // SAFETY: the pseudo handle of the current thread is always valid, and the
                // mask names one processor the process may use.
                let previous = unsafe { SetThreadAffinityMask(GetCurrentThread(), 1 << bit) };
                assert_ne!(previous, 0, "SetThreadAffinityMask to processor {bit}");
                let seen: Vec<_> = (0..100).map(|_| current_core_class()).collect();
                assert!(seen.iter().all(|&class| class == seen[0]), "processor {bit}: {seen:?}");
                let cpu = platform::current_cpu().expect("the processor number");
                assert_eq!(cpu % 64, bit);
                assert_eq!(seen[0], classes.get(&cpu).copied());
            }
        })
        .join()
        .unwrap();
    }
}
