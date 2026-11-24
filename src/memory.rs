//! One memory budget for the whole program, sized from the machine: how much
//! RAM it has and how much memory its graphics card has (and lets this
//! program use), and how tight memory is right now.
//!
//! The image budget (`Settings::cache_mb`) covers every decoded image kept as
//! a texture: full images, first views and grid thumbnails. *Memory use*
//! chooses it from the machine (Low / Balanced / High) or leaves it to the
//! user (Custom). While the program runs, `Guard` watches this process's
//! memory, the memory left to the whole system and the graphics card's use
//! against its budget, and answers with a `Pressure` level: Tight keeps fewer
//! images and pauses background work, Critical keeps only what is on screen.

use crate::model::MemoryUse;
use std::time::{Duration, Instant};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// What the machine has, read once at start (graphics card) and every second
/// (RAM left, graphics card use).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Machine {
    /// Physical RAM, bytes.
    pub ram_total: Option<u64>,
    /// RAM the system can still give out, bytes.
    pub ram_available: Option<u64>,
    /// The graphics card with the most memory of its own.
    pub gpu: Option<Gpu>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    pub name: String,
    /// Memory on the card itself, bytes (small on integrated graphics).
    pub dedicated: u64,
    /// What the system lets this program use on the card now, bytes.
    pub budget: Option<u64>,
    /// What this program uses on the card now, bytes.
    pub usage: Option<u64>,
}

impl Gpu {
    /// Integrated graphics share the system RAM: their own memory is only a
    /// small carve-out.
    pub fn shares_ram(&self) -> bool {
        self.dedicated < GIB
    }
}

/// The image budget in MiB for a choice of Memory use on this machine.
/// Custom keeps the user's value.
pub fn image_budget_mb(choice: MemoryUse, custom_mb: usize, machine: &Machine) -> usize {
    let (ram_share, gpu_share, low, high) = match choice {
        MemoryUse::Custom => return custom_mb.clamp(64, 8192),
        MemoryUse::Low => (0.05, 0.15, 128, 512),
        MemoryUse::Balanced => (0.10, 0.25, 256, 1536),
        MemoryUse::High => (0.20, 0.50, 512, 4096),
    };
    // Unknown RAM: assume a modest 8 GiB machine.
    let ram = machine.ram_total.unwrap_or(8 * GIB) as f64;
    let mut budget = ram * ram_share;
    if let Some(gpu) = machine.gpu.as_ref().filter(|g| !g.shares_ram()) {
        budget = budget.min(gpu.dedicated as f64 * gpu_share);
    }
    ((budget / MIB as f64) as usize).clamp(low, high)
}

/// How much memory this process may hold before memory counts as tight,
/// bytes: a share of the RAM, at least 2 GiB and room for twice the images.
pub fn process_limit(choice: MemoryUse, image_mb: usize, machine: &Machine) -> u64 {
    let share = match choice {
        MemoryUse::Low => 0.10,
        MemoryUse::Balanced | MemoryUse::Custom => 0.15,
        MemoryUse::High => 0.25,
    };
    let ram = machine.ram_total.unwrap_or(8 * GIB) as f64;
    ((ram * share) as u64)
        .max(2 * GIB)
        .max(2 * image_mb as u64 * MIB + GIB)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pressure {
    #[default]
    Normal,
    /// Half the image budget, fewer images ready ahead, background work paused.
    Tight,
    /// Only what is on screen, no animation, background work paused.
    Critical,
}

impl Pressure {
    /// The share of the image budget that may be used.
    pub fn budget_share(self) -> f64 {
        match self {
            Self::Normal => 1.,
            Self::Tight => 0.5,
            Self::Critical => 0.,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Tight => "tight",
            Self::Critical => "critical",
        }
    }
}

/// One reading of the memory situation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    /// This process's memory, bytes.
    pub process: Option<u64>,
    /// The limit for it (`process_limit`).
    pub limit: u64,
    pub ram_total: Option<u64>,
    pub ram_available: Option<u64>,
    pub gpu_usage: Option<u64>,
    pub gpu_budget: Option<u64>,
}

impl Reading {
    /// The level these numbers call for. `scale` < 1 asks for some room
    /// below the thresholds (to come back down only well below them).
    fn level(&self, scale: f64) -> Pressure {
        let ratio = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (Some(a), Some(b)) if b > 0 => Some(a as f64 / b as f64),
            _ => None,
        };
        let process = ratio(self.process, Some(self.limit));
        // The system's free RAM as a share of all of it.
        let free = ratio(self.ram_available, self.ram_total);
        let gpu = ratio(self.gpu_usage, self.gpu_budget);
        let over = |value: Option<f64>, limit: f64| value.is_some_and(|v| v > limit * scale);
        // Free RAM counts against the threshold the other way round.
        let short = |value: Option<f64>, limit: f64| value.is_some_and(|v| v < limit / scale);
        if over(process, 1.) || short(free, 0.05) || over(gpu, 0.95) {
            Pressure::Critical
        } else if over(process, 0.8) || short(free, 0.10) || over(gpu, 0.85) {
            Pressure::Tight
        } else {
            Pressure::Normal
        }
    }
}

/// Turns readings into a steady level: up at once, down only when the
/// numbers are well below the threshold for a few seconds.
#[derive(Debug, Default)]
pub struct Guard {
    pub level: Pressure,
    pub last: Reading,
    raised: Option<Instant>,
}

impl Guard {
    const HOLD: Duration = Duration::from_secs(5);
    /// Takes a reading; returns the new level when it changed.
    pub fn update(&mut self, reading: Reading, now: Instant) -> Option<Pressure> {
        self.last = reading;
        let wanted = reading.level(1.);
        let before = self.level;
        if wanted > self.level {
            self.level = wanted;
            self.raised = Some(now);
        } else if wanted < self.level
            && self
                .raised
                .is_none_or(|t| now.duration_since(t) >= Self::HOLD)
        {
            // Down one step at a time, and only with room to spare.
            let calm = reading.level(0.9);
            if calm < self.level {
                self.level = match self.level {
                    Pressure::Critical => Pressure::Tight,
                    _ => Pressure::Normal,
                };
                self.raised = Some(now);
            }
        }
        (self.level != before).then_some(self.level)
    }
}

/// Reads the machine's memory. The graphics card is looked up once and kept.
pub struct Probe {
    #[cfg(windows)]
    adapter: Option<windows::Win32::Graphics::Dxgi::IDXGIAdapter3>,
    gpu: Option<Gpu>,
}

impl Probe {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            let (adapter, gpu) = windows_gpu::find().unzip();
            Self { adapter, gpu }
        }
        #[cfg(not(windows))]
        Self { gpu: None }
    }
    /// The machine now.
    pub fn read(&self) -> Machine {
        let (ram_total, ram_available) = ram();
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut gpu = self.gpu.clone();
        #[cfg(windows)]
        if let (Some(adapter), Some(gpu)) = (&self.adapter, gpu.as_mut()) {
            if let Some((budget, usage)) = windows_gpu::use_of(adapter) {
                gpu.budget = Some(budget);
                gpu.usage = Some(usage);
            }
        }
        Machine {
            ram_total,
            ram_available,
            gpu,
        }
    }
}

/// Physical RAM and what is left of it, bytes.
fn ram() -> (Option<u64>, Option<u64>) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut status) != 0 {
            return (Some(status.ullTotalPhys), Some(status.ullAvailPhys));
        }
        (None, None)
    }
    #[cfg(target_os = "linux")]
    {
        let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
            return (None, None);
        };
        let field = |name: &str| {
            text.lines()
                .find(|l| l.starts_with(name))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
                .map(|kb| kb * 1024)
        };
        (field("MemTotal:"), field("MemAvailable:"))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        (None, None)
    }
}

#[cfg(windows)]
mod windows_gpu {
    use super::Gpu;
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIAdapter3, IDXGIFactory1,
        DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
    };

    /// The hardware graphics card with the most memory of its own (the one a
    /// desktop with a separate card draws on).
    pub fn find() -> Option<(IDXGIAdapter3, Gpu)> {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
        let mut best: Option<(IDXGIAdapter1, Gpu)> = None;
        for index in 0.. {
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
                break;
            };
            let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
                continue;
            };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let end = desc
                .Description
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(desc.Description.len());
            let gpu = Gpu {
                name: String::from_utf16_lossy(&desc.Description[..end]),
                dedicated: desc.DedicatedVideoMemory as u64,
                budget: None,
                usage: None,
            };
            if best
                .as_ref()
                .is_none_or(|(_, b)| gpu.dedicated > b.dedicated)
            {
                best = Some((adapter, gpu));
            }
        }
        let (adapter, gpu) = best?;
        Some((adapter.cast().ok()?, gpu))
    }

    /// (budget, usage) of this process on the card's own memory, bytes.
    pub fn use_of(adapter: &IDXGIAdapter3) -> Option<(u64, u64)> {
        let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        unsafe { adapter.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info) }
            .ok()?;
        Some((info.Budget, info.CurrentUsage))
    }
}

/// "1.5 GiB" / "640 MiB".
pub fn size(bytes: u64) -> String {
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{} MiB", bytes / MIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(ram_gib: u64, gpu_gib: Option<f64>) -> Machine {
        Machine {
            ram_total: Some(ram_gib * GIB),
            ram_available: Some(ram_gib * GIB / 2),
            gpu: gpu_gib.map(|g| Gpu {
                name: "card".into(),
                dedicated: (g * GIB as f64) as u64,
                budget: None,
                usage: None,
            }),
        }
    }

    #[test]
    fn the_image_budget_follows_the_machine_and_the_choice() {
        let desktop = machine(64, Some(16.));
        assert_eq!(image_budget_mb(MemoryUse::Low, 0, &desktop), 512);
        assert_eq!(image_budget_mb(MemoryUse::Balanced, 0, &desktop), 1536);
        assert_eq!(image_budget_mb(MemoryUse::High, 0, &desktop), 4096);
        // A laptop with integrated graphics: RAM only.
        let laptop = machine(8, Some(0.5));
        assert_eq!(image_budget_mb(MemoryUse::Balanced, 0, &laptop), 819);
        assert_eq!(image_budget_mb(MemoryUse::Low, 0, &laptop), 409);
        // A small separate card limits it.
        let small_card = machine(32, Some(2.));
        assert_eq!(image_budget_mb(MemoryUse::Balanced, 0, &small_card), 512);
        assert_eq!(image_budget_mb(MemoryUse::Low, 0, &small_card), 307);
        // Nothing known: as 8 GiB of RAM.
        assert_eq!(
            image_budget_mb(MemoryUse::Balanced, 0, &Machine::default()),
            819
        );
        assert_eq!(image_budget_mb(MemoryUse::Custom, 3000, &desktop), 3000);
        assert_eq!(image_budget_mb(MemoryUse::Custom, 10, &desktop), 64);
    }

    #[test]
    fn the_process_limit_is_a_share_of_ram_with_room_for_the_images() {
        let desktop = machine(64, Some(16.));
        assert_eq!(
            process_limit(MemoryUse::Balanced, 1536, &desktop),
            (64. * 0.15 * GIB as f64) as u64
        );
        assert_eq!(
            process_limit(MemoryUse::Low, 512, &machine(8, None)),
            2 * GIB
        );
        assert_eq!(
            process_limit(MemoryUse::Custom, 4096, &machine(8, None)),
            9 * GIB
        );
    }

    fn reading(process_gib: f64, free_share: f64, gpu_share: Option<f64>) -> Reading {
        Reading {
            process: Some((process_gib * GIB as f64) as u64),
            limit: 10 * GIB,
            ram_total: Some(100 * GIB),
            ram_available: Some((free_share * 100. * GIB as f64) as u64),
            gpu_usage: gpu_share.map(|s| (s * 1000.) as u64),
            gpu_budget: gpu_share.map(|_| 1000),
        }
    }

    #[test]
    fn pressure_rises_at_once_and_falls_slowly_with_room_to_spare() {
        let mut guard = Guard::default();
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        assert_eq!(guard.update(reading(2., 0.5, Some(0.5)), at(0)), None);
        // Each signal on its own.
        assert_eq!(
            guard.update(reading(8.5, 0.5, None), at(1)),
            Some(Pressure::Tight)
        );
        assert_eq!(
            guard.update(reading(2., 0.04, None), at(2)),
            Some(Pressure::Critical)
        );
        // Calm again: held for five seconds after the last rise.
        assert_eq!(guard.update(reading(2., 0.5, Some(0.2)), at(4)), None);
        assert_eq!(
            guard.update(reading(2., 0.5, Some(0.2)), at(7)),
            Some(Pressure::Tight)
        );
        // Just under the threshold is not room enough to come down.
        assert_eq!(guard.update(reading(7.9, 0.5, None), at(13)), None);
        assert_eq!(guard.level, Pressure::Tight);
        assert_eq!(
            guard.update(reading(7., 0.5, None), at(14)),
            Some(Pressure::Normal)
        );
        // The graphics card near its budget.
        assert_eq!(
            guard.update(reading(2., 0.5, Some(0.9)), at(15)),
            Some(Pressure::Tight)
        );
        assert_eq!(
            guard.update(reading(2., 0.5, Some(0.97)), at(16)),
            Some(Pressure::Critical)
        );
        assert_eq!(Pressure::Tight.budget_share(), 0.5);
    }

    #[test]
    fn the_machine_is_read() {
        let machine = Probe::new().read();
        #[cfg(any(windows, target_os = "linux"))]
        {
            assert!(machine.ram_total.unwrap() > 256 * MIB);
            assert!(machine.ram_available.unwrap() <= machine.ram_total.unwrap());
        }
        let _ = machine;
    }
}
