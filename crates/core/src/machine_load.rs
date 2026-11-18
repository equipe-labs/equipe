//! How busy the machine equipe runs on is, and whether new work waits for it
//! (docs/LOAD.md).
//!
//! A Mac shared with other work (a CI runner, a dozen headless browsers, a
//! member's whole-disk search) can be pushed far past what it can do: a load
//! average of 60 on 15 cores, a few megabytes of memory left. Starting more
//! there makes everything slower, the daemon's own answers included. So while
//! the machine stays overloaded, equipe starts no new work of its own (ticket
//! runs, routines, the PM's idle wakes, design checks' browsers) and says so;
//! once it calms down, it carries on by itself. Turns already running are never
//! stopped.
//!
//! Read once a minute, cheaply: `getloadavg` and, on macOS, one `sysctl` —
//! no process started, which on an overloaded Mac can take seconds itself.

/// What the machine looks like now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// The load average over the last minute…
    pub load: f64,
    /// …and over the last five: against it, whether the load rises or falls.
    pub load5: f64,
    /// Logical cores.
    pub cores: usize,
    /// How much memory is left to use, in percent (macOS's memory pressure
    /// level, Linux's `MemAvailable`); `None` when it can't be told.
    pub memory_left: Option<u8>,
}

/// Overloaded past this many runnable threads per core while the load still
/// rises: a Mac shared with a CI runner and a dozen browsers lives at 4-5x
/// its cores, and a line at 1.5x never let new work start there…
pub const LOAD_PER_CORE: f64 = 2.5;
/// …and calm again once the load falls, or is under this many.
pub const CALM_PER_CORE: f64 = 2.0;
/// Memory this low (percent left) is overloaded whatever the load…
pub const MEMORY_CRITICAL: u8 = 5;
/// …and has room again above this.
pub const MEMORY_ROOM: u8 = 10;
/// Readings in a row (a minute apart) before the level changes: a moment's
/// spike — a build linking — holds nothing back.
pub const READINGS_TO_CHANGE: u8 = 2;

/// Why the machine counts as overloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Load,
    Memory,
}

impl Reading {
    /// Why this reading is over the line, if it is: memory nearly gone, or
    /// a load past [`LOAD_PER_CORE`] that is still rising.
    pub fn over(&self) -> Option<Why> {
        if self.memory_left.is_some_and(|m| m <= MEMORY_CRITICAL) {
            return Some(Why::Memory);
        }
        (self.load > LOAD_PER_CORE * self.cores.max(1) as f64 && self.load >= self.load5).then_some(Why::Load)
    }

    /// Whether this reading is calm enough to carry on: memory to spare, and
    /// a load that falls or is under [`CALM_PER_CORE`].
    pub fn calm(&self) -> bool {
        let falling = self.load < self.load5;
        (falling || self.load < CALM_PER_CORE * self.cores.max(1) as f64) && self.memory_left.is_none_or(|m| m > MEMORY_ROOM)
    }

    /// The load, rounded, for people: "60".
    pub fn load_shown(&self) -> String {
        format!("{:.0}", self.load)
    }
}

/// What changed with a reading ([`Watch::observe`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    /// It just became overloaded: new work waits.
    Overloaded(Reading, Why),
    /// It calmed down: what waited starts.
    Calm(Reading),
}

/// The machine's level, from one reading to the next.
#[derive(Debug, Default)]
pub struct Watch {
    overloaded: bool,
    /// Readings in a row that point the other way.
    against: u8,
    pub last: Option<Reading>,
}

impl Watch {
    /// Whether new work waits now.
    pub fn overloaded(&self) -> bool {
        self.overloaded
    }

    /// A fresh reading: `Some` when the level changes with it.
    pub fn observe(&mut self, reading: Reading) -> Option<Change> {
        self.last = Some(reading);
        let why = reading.over();
        let against = if self.overloaded { reading.calm() } else { why.is_some() };
        if !against {
            self.against = 0;
            return None;
        }
        self.against += 1;
        if self.against < READINGS_TO_CHANGE {
            return None;
        }
        self.against = 0;
        self.overloaded = !self.overloaded;
        Some(match why {
            Some(why) if self.overloaded => Change::Overloaded(reading, why),
            _ => Change::Calm(reading),
        })
    }
}

/// The machine now; `None` where it can't be read.
pub fn read() -> Option<Reading> {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let (load, load5) = load_average()?;
    Some(Reading { load, load5, cores, memory_left: memory_left() })
}

#[cfg(unix)]
fn load_average() -> Option<(f64, f64)> {
    let mut loads = [0f64; 3];
    // SAFETY: `loads` has room for the three values asked for.
    let n = unsafe { libc::getloadavg(loads.as_mut_ptr(), 3) };
    (n >= 2).then_some((loads[0], loads[1]))
}

#[cfg(not(unix))]
fn load_average() -> Option<(f64, f64)> {
    None
}

/// macOS: `kern.memorystatus_level`, the share of memory still available
/// as the system counts it (what Activity Monitor's pressure graph shows).
#[cfg(target_os = "macos")]
fn memory_left() -> Option<u8> {
    let mut level: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let name = c"kern.memorystatus_level";
    // SAFETY: `level` and `size` are valid for the int the name reads into.
    let ok = unsafe { libc::sysctlbyname(name.as_ptr(), (&raw mut level).cast(), &raw mut size, std::ptr::null_mut(), 0) };
    (ok == 0).then(|| level.clamp(0, 100) as u8)
}

#[cfg(target_os = "linux")]
fn memory_left() -> Option<u8> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    memory_left_of(&info)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn memory_left() -> Option<u8> {
    None
}

/// `MemAvailable` of `MemTotal`, in percent, from `/proc/meminfo`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn memory_left_of(meminfo: &str) -> Option<u8> {
    let kb = |key: &str| meminfo.lines().find(|l| l.starts_with(key))?.split_whitespace().nth(1)?.parse::<u64>().ok();
    let (available, total) = (kb("MemAvailable:")?, kb("MemTotal:")?);
    (total > 0).then(|| (available.saturating_mul(100) / total).min(100) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reading on 15 cores: the load now, and five minutes' average.
    fn reading(load: f64, load5: f64, memory_left: Option<u8>) -> Reading {
        Reading { load, load5, cores: 15, memory_left }
    }

    #[test]
    fn overloaded_only_while_the_load_stays_high_and_rising_and_calm_once_it_falls() {
        let mut watch = Watch::default();
        // A Mac that lives at 4x its cores, steady or falling: work goes on.
        for (now, five) in [(60.0, 62.0), (58.0, 61.0), (55.0, 60.0)] {
            assert_eq!(watch.observe(reading(now, five, Some(40))), None);
        }
        assert!(!watch.overloaded());
        // Past 2.5x and rising, twice in a row: new work waits.
        assert_eq!(watch.observe(reading(70.0, 60.0, Some(40))), None);
        assert_eq!(watch.observe(reading(77.0, 65.0, Some(40))), Some(Change::Overloaded(reading(77.0, 65.0, Some(40)), Why::Load)));
        assert!(watch.overloaded());
        // Still high and rising: held.
        assert_eq!(watch.observe(reading(78.0, 70.0, Some(40))), None);
        // Falling twice in a row, though still high: what waited starts.
        assert_eq!(watch.observe(reading(72.0, 74.0, Some(40))), None);
        assert_eq!(watch.observe(reading(68.0, 72.0, Some(40))), Some(Change::Calm(reading(68.0, 72.0, Some(40)))));
        assert!(!watch.overloaded());
        // The old line (1.5x) holds nothing back any more.
        assert_eq!(reading(30.0, 25.0, Some(40)).over(), None);
    }

    #[test]
    fn memory_nearly_gone_is_overloaded_whatever_the_load() {
        let mut watch = Watch::default();
        watch.observe(reading(3.0, 3.0, Some(4)));
        assert_eq!(watch.observe(reading(3.0, 3.0, Some(3))), Some(Change::Overloaded(reading(3.0, 3.0, Some(3)), Why::Memory)));
        // Calm load but memory still short: it waits on.
        watch.observe(reading(3.0, 3.0, Some(8)));
        assert_eq!(watch.observe(reading(3.0, 3.0, Some(8))), None);
        watch.observe(reading(3.0, 3.0, Some(30)));
        assert!(matches!(watch.observe(reading(3.0, 3.0, Some(30))), Some(Change::Calm(_))));
        // Unknown memory never holds anything.
        assert_eq!(reading(3.0, 3.0, None).over(), None);
    }

    #[test]
    fn linux_memory_left_is_available_of_total() {
        let info = "MemTotal:       16000000 kB\nMemFree:          100000 kB\nMemAvailable:     800000 kB\n";
        assert_eq!(memory_left_of(info), Some(5));
        assert_eq!(memory_left_of("MemTotal: 0 kB\nMemAvailable: 0 kB\n"), None);
    }

    #[test]
    fn this_machine_can_be_read() {
        if cfg!(unix) {
            let now = read().expect("a load average on unix");
            assert!(now.cores >= 1 && now.load >= 0.0 && now.load5 >= 0.0);
        }
    }
}
