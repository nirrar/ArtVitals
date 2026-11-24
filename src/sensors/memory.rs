//! Odczyt zajetosci pamieci operacyjnej (RAM) przez GlobalMemoryStatusEx.
//! Nie wymaga uprawnien administratora. Zaleznosci: windows (Win32_System_SystemInformation).

use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryReading {
    pub usage_pct: Option<f32>,
    pub used_gb: Option<f32>,
    pub total_gb: Option<f32>,
}

const BYTES_PER_GB: f64 = 1024.0 * 1024.0 * 1024.0;

pub fn read() -> MemoryReading {
    let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut status) }.is_err() || status.ullTotalPhys == 0 {
        return MemoryReading::default();
    }
    let total = status.ullTotalPhys as f64;
    let used = total - status.ullAvailPhys as f64;
    MemoryReading {
        usage_pct: Some((used / total * 100.0) as f32),
        used_gb: Some((used / BYTES_PER_GB) as f32),
        total_gb: Some((total / BYTES_PER_GB) as f32),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_plausible_values() {
        let reading = super::read();
        let pct = reading.usage_pct.unwrap();
        assert!((0.0..=100.0).contains(&pct));
        assert!(reading.used_gb.unwrap() <= reading.total_gb.unwrap());
    }
}
