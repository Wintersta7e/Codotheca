//! §24.4's six stages, entered on real milestones parsed from the child's **stderr**.
//!
//! `git clone --progress` writes progress to stderr when it is not attached to a terminal, which
//! is why `Intent::Clone` carries `--progress` (`core/src/gitw/intent.rs`). The core reads that
//! stream and never forwards it to its own stdout, which carries protocol frames and nothing else.
//!
//! **`done` and `total` are per-phase and both nullable, and there is no aggregate field by
//! construction** (A10). A phase with no denominator sends `total: null` and the surface renders a
//! bare count. That is what makes §10.2's ban on a retreating percentage provable rather than a
//! style rule: there is nothing on the wire to build one from.
//!
//! **The machine never retreats.** It loops in place until the next milestone arrives, so a
//! readout can only ever move forward — and it never skips: a transcript that jumps straight from
//! `enumerating` to `cladding` still emits `receiving` and `assembling`, because a stage the user
//! was shown and then never saw again is a stage that appeared to fail.

use crate::protocol::InstallStageKind;

/// One parsed stderr line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageObservation {
    /// The §24.4 stage the line's milestone belongs to.
    pub kind: InstallStageKind,
    /// Items done **within this phase**.
    pub done: Option<i64>,
    /// This phase's denominator, or `None` when it has none.
    pub total: Option<i64>,
    /// Bytes received, where git reports them.
    pub bytes: Option<i64>,
}

/// §24.4's stage order. The machine walks this and never indexes backwards.
const ORDER: [InstallStageKind; 6] = [
    InstallStageKind::Plans,
    InstallStageKind::Enumerating,
    InstallStageKind::Receiving,
    InstallStageKind::Assembling,
    InstallStageKind::Cladding,
    InstallStageKind::Settled,
];

fn rank(kind: InstallStageKind) -> usize {
    ORDER.iter().position(|k| *k == kind).unwrap_or(0)
}

/// Read one `KiB`/`MiB`/`GiB` figure into bytes.
///
/// **Integer arithmetic throughout, with no float and no `as` cast.** git prints at most two
/// decimal places, so the fraction is scaled by its own digit count rather than multiplied out
/// in `f64` — which keeps the result exact and keeps `cast_possible_truncation` from being
/// silenced on a value the user reads.
fn parse_bytes(text: &str) -> Option<i64> {
    let (number, unit) = text.split_once(' ').or_else(|| {
        let idx = text.find(|c: char| c.is_ascii_alphabetic())?;
        text.split_at_checked(idx)
    })?;
    let scale: i64 = match unit.trim() {
        "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        "GiB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    let (whole, fraction) = number
        .trim()
        .split_once('.')
        .unwrap_or_else(|| (number.trim(), ""));
    let whole: i64 = whole.parse().ok()?;
    let mut bytes = whole.checked_mul(scale)?;
    if !fraction.is_empty() {
        let digits: u32 = u32::try_from(fraction.len()).ok()?;
        let numerator: i64 = fraction.parse().ok()?;
        let divisor = 10_i64.checked_pow(digits)?;
        bytes = bytes.checked_add(numerator.checked_mul(scale)? / divisor)?;
    }
    Some(bytes)
}

/// Parse one line of `git clone --progress` stderr.
///
/// Returns `None` for a line that names no milestone — which is most of them. A parser that
/// guessed a stage from an unrecognised line would advance the readout on noise.
#[must_use]
pub fn parse_progress_line(line: &str) -> Option<StageObservation> {
    let line = line.trim();
    let (label, rest) = line.split_once(':')?;
    let kind = match label.trim() {
        "Enumerating objects" | "Counting objects" => InstallStageKind::Enumerating,
        "Receiving objects" => InstallStageKind::Receiving,
        "Resolving deltas" => InstallStageKind::Assembling,
        "Updating files" | "Checking out files" => InstallStageKind::Cladding,
        _ => return None,
    };

    let mut done = None;
    let mut total = None;
    let mut bytes = None;
    for part in rest.split(',') {
        let part = part.trim();
        if let Some((_, opened)) = part.split_once('(') {
            // `Receiving objects:  73% (2196/3007), 2.14 MiB | 1.07 MiB/s`
            if let Some((inner, _)) = opened.split_once(')') {
                if let Some((a, b)) = inner.split_once('/') {
                    done = a.trim().parse().ok();
                    total = b.trim().parse().ok();
                }
            }
        } else if part.contains("iB") || part.ends_with(" B") {
            // A rate reads `1.07 MiB/s`; only the size term is a byte count.
            let size = part.split('|').next().unwrap_or(part).trim();
            if !size.contains("/s") {
                bytes = parse_bytes(size);
            }
        } else if done.is_none() {
            // `Enumerating objects: 3007` — a count with no denominator.
            if let Ok(value) = part.trim_end_matches('.').trim().parse::<i64>() {
                done = Some(value);
            }
        }
    }
    Some(StageObservation {
        kind,
        done,
        total,
        bytes,
    })
}

/// Walks §24.4's stages forward, never back and never past one.
#[derive(Debug, Clone)]
pub struct StageMachine {
    current: InstallStageKind,
    /// The highest `done` seen inside the current phase, so a retreating figure is clamped
    /// rather than published — §10.2 bans a number that goes backwards, and git does re-report.
    high_water: Option<i64>,
}

impl Default for StageMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl StageMachine {
    /// A machine at `plans`, the stage every run starts in, with no figure seen yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current: InstallStageKind::Plans,
            high_water: None,
        }
    }

    /// The stage the run is in now: the last one entered.
    #[must_use]
    pub const fn current(&self) -> InstallStageKind {
        self.current
    }

    /// Advance on one observation.
    ///
    /// Returns **every** stage entered, in order, because a jump forward still has to pass
    /// through the stages it skipped: a readout that showed `enumerating` and then `cladding`
    /// tells the user two phases failed. An observation for the current or an earlier stage
    /// advances nothing and returns an empty list.
    pub fn advance(&mut self, observation: StageObservation) -> Vec<StageObservation> {
        let target = rank(observation.kind);
        let here = rank(self.current);
        if self.current == InstallStageKind::Settled {
            // Terminal. Looping in place is right for a phase whose figure still moves; it is
            // wrong for the end, where a second emission would tell a surface the run settled
            // twice — and §24.4 reports `settled` so a readout never has to infer the end from
            // silence, which only works if it arrives exactly once.
            return Vec::new();
        }
        if target <= here {
            // Loop in place. The figure may still move, but only upwards.
            if observation.kind == self.current {
                let clamped = self.clamp(observation);
                return vec![clamped];
            }
            return Vec::new();
        }
        let mut entered = Vec::new();
        for (step, kind) in ORDER.iter().enumerate().take(target + 1).skip(here + 1) {
            let kind = *kind;
            self.current = kind;
            self.high_water = None;
            if step == target {
                entered.push(self.clamp(observation));
            } else {
                // A stage passed through was never observed, so it carries no figures: a count
                // invented for it would be a number the user could read as measured.
                entered.push(StageObservation {
                    kind,
                    done: None,
                    total: None,
                    bytes: None,
                });
            }
        }
        entered
    }

    /// The terminal stage, entered when the `location` row commits.
    pub fn settle(&mut self) -> Vec<StageObservation> {
        self.advance(StageObservation {
            kind: InstallStageKind::Settled,
            done: None,
            total: None,
            bytes: None,
        })
    }

    /// Never let a figure go backwards inside one phase.
    fn clamp(&mut self, mut observation: StageObservation) -> StageObservation {
        match (self.high_water, observation.done) {
            (Some(high), Some(done)) if done < high => observation.done = Some(high),
            (_, Some(done)) => self.high_water = Some(done),
            _ => {}
        }
        observation
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{parse_progress_line, StageMachine, StageObservation};
    use crate::protocol::InstallStageKind;

    #[test]
    fn a_line_naming_no_milestone_advances_nothing() {
        assert!(parse_progress_line("Cloning into 'widget'...").is_none());
        assert!(parse_progress_line("").is_none());
        assert!(parse_progress_line("remote: Total 3007 (delta 0)").is_none());
    }

    #[test]
    fn a_phase_with_no_denominator_reports_a_bare_count() {
        let observed = parse_progress_line("Enumerating objects: 3007, done.").expect("parsed");
        assert_eq!(observed.kind, InstallStageKind::Enumerating);
        assert_eq!(observed.done, Some(3007));
        assert_eq!(
            observed.total, None,
            "no denominator exists, so none is invented"
        );
    }

    #[test]
    fn receiving_carries_its_denominator_and_its_bytes() {
        let observed =
            parse_progress_line("Receiving objects:  73% (2196/3007), 2.14 MiB | 1.07 MiB/s")
                .expect("parsed");
        assert_eq!(observed.kind, InstallStageKind::Receiving);
        assert_eq!(observed.done, Some(2196));
        assert_eq!(observed.total, Some(3007));
        assert_eq!(observed.bytes, Some(2_243_952));
    }

    #[test]
    fn the_machine_never_skips_a_stage_even_when_the_transcript_does() {
        let mut machine = StageMachine::new();
        let entered = machine.advance(StageObservation {
            kind: InstallStageKind::Cladding,
            done: Some(1),
            total: Some(1),
            bytes: None,
        });
        let kinds: Vec<_> = entered.iter().map(|o| o.kind).collect();
        assert_eq!(
            kinds,
            vec![
                InstallStageKind::Enumerating,
                InstallStageKind::Receiving,
                InstallStageKind::Assembling,
                InstallStageKind::Cladding
            ],
            "a stage shown and then never seen again reads as a phase that failed"
        );
        // The stages passed through carry no figures, because none were observed for them.
        assert_eq!(entered[0].done, None);
        assert_eq!(entered[3].done, Some(1));
    }

    #[test]
    fn the_machine_never_retreats() {
        let mut machine = StageMachine::new();
        machine.advance(StageObservation {
            kind: InstallStageKind::Receiving,
            done: Some(10),
            total: Some(100),
            bytes: None,
        });
        let back = machine.advance(StageObservation {
            kind: InstallStageKind::Enumerating,
            done: Some(3),
            total: None,
            bytes: None,
        });
        assert!(back.is_empty(), "an earlier stage publishes nothing");
        assert_eq!(machine.current(), InstallStageKind::Receiving);
    }

    #[test]
    fn a_figure_never_decreases_within_a_phase() {
        let mut machine = StageMachine::new();
        machine.advance(StageObservation {
            kind: InstallStageKind::Receiving,
            done: Some(900),
            total: Some(1000),
            bytes: None,
        });
        let again = machine.advance(StageObservation {
            kind: InstallStageKind::Receiving,
            done: Some(400),
            total: Some(1000),
            bytes: None,
        });
        assert_eq!(
            again[0].done,
            Some(900),
            "§10.2 bans a number that goes backwards, and git does re-report"
        );
    }

    #[test]
    fn settling_is_terminal_and_reached_from_wherever_the_run_was() {
        let mut machine = StageMachine::new();
        let entered = machine.settle();
        assert_eq!(machine.current(), InstallStageKind::Settled);
        assert_eq!(
            entered.last().map(|o| o.kind),
            Some(InstallStageKind::Settled)
        );
        assert!(machine.settle().is_empty(), "settled is entered once");
    }
}
