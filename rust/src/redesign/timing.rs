use std::ffi::{c_int, c_long};
use std::time::{Duration, Instant};

#[repr(C)]
#[derive(Default)]
struct Timespec {
    seconds: c_long,
    nanos: c_long,
}
unsafe extern "C" {
    fn clock_gettime(clock: c_int, time: *mut Timespec) -> c_int;
}
fn process_cpu() -> Duration {
    let mut time = Timespec::default();
    // Linux CLOCK_PROCESS_CPUTIME_ID includes user and system CPU for all threads.
    let result = unsafe { clock_gettime(2, &mut time) };
    assert_eq!(result, 0);
    Duration::new(time.seconds as u64, time.nanos as u32)
}

#[derive(Clone, Copy)]
pub enum Stage {
    Parse,
    Links,
    Dedup,
    Resolve,
    Aggregate,
    Write,
    Setup,
}
pub struct Timings {
    enabled: bool,
    last_wall: Instant,
    last_cpu: Duration,
    times: [(Duration, Duration); 7],
}
impl Timings {
    pub fn new() -> Self {
        let enabled = std::env::var("SHOP_TIMING").as_deref() == Ok("1");
        Self {
            enabled,
            last_wall: Instant::now(),
            last_cpu: if enabled {
                process_cpu()
            } else {
                Duration::ZERO
            },
            times: Default::default(),
        }
    }
    pub fn end(&mut self, stage: Stage) {
        if self.enabled {
            let now = Instant::now();
            let cpu = process_cpu();
            let (wall_total, cpu_total) = &mut self.times[stage as usize];
            *wall_total += now - self.last_wall;
            *cpu_total += cpu - self.last_cpu;
            self.last_wall = now;
            self.last_cpu = cpu;
        }
    }
    pub fn print(&self) {
        if self.enabled {
            for (label, (wall, cpu)) in [
                "parse",
                "links",
                "dedup",
                "resolve",
                "aggregate",
                "parquet",
                "setup",
            ]
            .into_iter()
            .zip(self.times)
            {
                eprintln!(
                    "{label} wall={:.6} cpu={:.6}",
                    wall.as_secs_f64(),
                    cpu.as_secs_f64()
                );
            }
        }
    }
}
