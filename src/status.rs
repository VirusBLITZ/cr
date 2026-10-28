//! Runs jobs in parallel while showing one live-updating status line per job.

use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const TICK: Duration = Duration::from_millis(100);

/// `None` while running, then whether it succeeded and how long it took.
type State = Option<(bool, Duration)>;

/// Runs every job on its own thread and returns their results in order.
/// On a terminal the status block is redrawn in place; otherwise each job
/// prints one line when it finishes.
pub fn run<T, F>(names: &[String], jobs: Vec<F>) -> Vec<Result<T>>
where
    T: Send,
    F: FnOnce() -> Result<T> + Send,
{
    let started = Instant::now();
    let states: Mutex<Vec<State>> = Mutex::new(vec![None; jobs.len()]);
    let width = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
    let tty = std::io::stderr().is_terminal();

    std::thread::scope(|s| {
        let handles: Vec<_> = jobs
            .into_iter()
            .enumerate()
            .map(|(i, job)| {
                let states = &states;
                s.spawn(move || {
                    let result = job();
                    let state = Some((result.is_ok(), started.elapsed()));
                    states.lock().unwrap()[i] = state;
                    if !tty {
                        eprintln!("{}", line(&names[i], width, state, started));
                    }
                    result
                })
            })
            .collect();

        if tty {
            let mut stderr = std::io::stderr().lock();
            for frame in 0.. {
                let snapshot = states.lock().unwrap().clone();
                if frame > 0 {
                    write!(stderr, "\x1b[{}A", names.len()).ok();
                }
                for (name, state) in names.iter().zip(&snapshot) {
                    write!(stderr, "\r\x1b[2K{}\n", line(name, width, *state, started)).ok();
                }
                stderr.flush().ok();
                // Also stop if a job panicked and never reported its state.
                if snapshot.iter().all(Option::is_some) || handles.iter().all(|h| h.is_finished()) {
                    break;
                }
                std::thread::sleep(TICK);
            }
        }

        handles.into_iter().map(|h| h.join().expect("job thread panicked")).collect()
    })
}

/// Runs a single job with a status line.
pub fn run_one<T: Send>(name: String, job: impl FnOnce() -> Result<T> + Send) -> Result<T> {
    run(&[name], vec![job]).pop().expect("one job, one result")
}

fn line(name: &str, width: usize, state: State, started: Instant) -> String {
    let (icon, label, elapsed) = match state {
        None => {
            let elapsed = started.elapsed();
            let frame = (elapsed.as_millis() / TICK.as_millis()) as usize % SPINNER.len();
            (SPINNER[frame], "running", elapsed)
        }
        Some((true, elapsed)) => ('✓', "done", elapsed),
        Some((false, elapsed)) => ('✗', "failed", elapsed),
    };
    let secs = elapsed.as_secs();
    format!("{icon} {name:<width$}  {label:<7} {}:{:02}", secs / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_results_in_order() {
        let names = ["slow".to_string(), "fails".to_string(), "fast".to_string()];
        let jobs: Vec<Box<dyn FnOnce() -> Result<u32> + Send>> = vec![
            Box::new(|| {
                std::thread::sleep(Duration::from_millis(300));
                Ok(1)
            }),
            Box::new(|| {
                std::thread::sleep(Duration::from_millis(150));
                anyhow::bail!("boom")
            }),
            Box::new(|| Ok(3)),
        ];
        let results = run(&names, jobs);
        assert_eq!(results[0].as_ref().unwrap(), &1);
        assert!(results[1].is_err());
        assert_eq!(results[2].as_ref().unwrap(), &3);
    }
}
