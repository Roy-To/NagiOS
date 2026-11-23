//! Development Notes telemetry, using kernel and presentation timestamps from CLOCK_MONOTONIC.

const NANOS_PER_SECOND: u64 = 1_000_000_000;
const NANOS_PER_MILLISECOND: u64 = 1_000_000;

pub(crate) fn evdev_monotonic_ns(record: &[u8; 24]) -> Option<u64> {
    // The x86-64 Linux input_event ABI begins with two native-endian i64s.
    let seconds = i64::from_le_bytes(record[..8].try_into().ok()?);
    let micros = i64::from_le_bytes(record[8..16].try_into().ok()?);
    if !(0..1_000_000).contains(&micros) {
        return None;
    }
    timestamp_ns(seconds, micros * 1_000).filter(|ns| *ns != 0)
}

pub(crate) fn clock_setup_dropped(ready_at: Option<u64>, record: &[u8; 24]) -> bool {
    // Only pre-console loss can be disregarded. A later overflow stays fatal,
    // and an untrusted or unavailable clock cannot establish this exception.
    ready_at
        .zip(evdev_monotonic_ns(record))
        .is_some_and(|(ready, dropped)| dropped <= ready)
}

fn timestamp_ns(seconds: i64, nanos: i64) -> Option<u64> {
    let nanos = u64::try_from(nanos)
        .ok()
        .filter(|ns| *ns < NANOS_PER_SECOND)?;
    u64::try_from(seconds)
        .ok()?
        .checked_mul(NANOS_PER_SECOND)?
        .checked_add(nanos)
}

pub(crate) fn presentation_record(
    received_at: Option<u64>,
    presented_at: Option<u64>,
) -> Option<String> {
    let ns = presented_at?.checked_sub(received_at?)?;
    Some(format!("NAGI_A_INPUT_TO_PRESENT ns={ns}"))
}

fn memory_record(now_ns: u64, meminfo: &str) -> Option<String> {
    let read_kib = |label: &str| {
        let mut matches = meminfo.lines().filter_map(|line| line.strip_prefix(label));
        let mut fields = matches.next()?.split_ascii_whitespace();
        let value = fields.next()?;
        if !value.bytes().all(|byte| byte.is_ascii_digit())
            || fields.next()? != "kB"
            || fields.next().is_some()
            || matches.next().is_some()
        {
            return None;
        }
        value.parse::<u64>().ok()
    };
    let total = read_kib("MemTotal:")?;
    let available = read_kib("MemAvailable:")?;
    if total == 0 || available > total {
        return None;
    }
    let t_ms = now_ns / NANOS_PER_MILLISECOND;
    Some(format!(
        "NAGI_A_GUEST_MEM t_ms={t_ms} mem_total_kib={total} mem_available_kib={available}"
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn monotonic_ns() -> Option<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time is writable for the duration of clock_gettime.
    (unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } == 0)
        .then(|| timestamp_ns(time.tv_sec, time.tv_nsec))
        .flatten()
}

#[cfg(target_os = "linux")]
#[derive(Default)]
pub(crate) struct MemorySampler {
    next_sample_ns: u64,
}

#[cfg(target_os = "linux")]
impl MemorySampler {
    pub(crate) fn sample_if_due(&mut self) -> Option<String> {
        let now = monotonic_ns()?;
        if now < self.next_sample_ns {
            return None;
        }
        self.next_sample_ns = now.saturating_add(NANOS_PER_SECOND);
        memory_record(now, &std::fs::read_to_string("/proc/meminfo").ok()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_record(seconds: i64, micros: i64) -> [u8; 24] {
        let mut record = [0; 24];
        record[..8].copy_from_slice(&seconds.to_le_bytes());
        record[8..16].copy_from_slice(&micros.to_le_bytes());
        record
    }

    #[test]
    fn kernel_timestamp_is_validated_before_latency_is_recorded() {
        let received = evdev_monotonic_ns(&input_record(3, 100));
        assert_eq!(received, Some(3_000_100_000));
        let record = presentation_record(received, Some(3_000_115_000)).unwrap();
        assert_eq!(record, "NAGI_A_INPUT_TO_PRESENT ns=15000");
        println!("{record}");
        assert!(presentation_record(None, Some(3_000_115_000)).is_none());
        assert!(presentation_record(received, None).is_none());
        assert!(presentation_record(received, Some(3_000_099_999)).is_none());
        for (seconds, micros) in [(-1, 10), (0, 0), (1, -1), (1, 1_000_000), (i64::MAX, 0)] {
            assert_eq!(evdev_monotonic_ns(&input_record(seconds, micros)), None);
        }
    }

    #[test]
    fn only_loss_during_clock_setup_is_ignored() {
        let ready = Some(3_000_115_000);
        assert!(clock_setup_dropped(ready, &input_record(3, 114)));
        assert!(!clock_setup_dropped(ready, &input_record(3, 116)));
        assert!(!clock_setup_dropped(ready, &input_record(0, 0)));
        assert!(!clock_setup_dropped(None, &input_record(3, 114)));
    }

    #[test]
    fn memory_sample_contains_only_the_two_expected_counters() {
        let record = memory_record(
            3_550_004_123,
            "MemTotal:       2097152 kB\nMemFree: 25 kB\nMemAvailable:    1800000 kB\n",
        )
        .unwrap();
        assert_eq!(
            record,
            "NAGI_A_GUEST_MEM t_ms=3550 mem_total_kib=2097152 mem_available_kib=1800000"
        );
        println!("{record}");
        for invalid in [
            "MemTotal: 100 kB\n",
            "MemTotal: 100 kB\nMemAvailable: 101 kB\n",
            "MemTotal: 100 kB\nMemAvailable: 10 MB\n",
            "MemTotal: 100 kB\nMemAvailable: 10 kB\nMemAvailable: 20 kB\n",
        ] {
            assert!(memory_record(1_000_000, invalid).is_none());
        }
    }
}
