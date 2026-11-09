//! The state the timeline puts the door in at a given time.

use protocol::Timeline;

use crate::Mode;

/// The soonest time after `now`, in Unix seconds, at which the state the
/// timeline puts the door in may change: a span starting or ending, or the
/// timeline ending; `None` when none is still to come.
pub(crate) fn next_change(timeline: &Timeline, now: i64) -> Option<i64> {
    let edges = timeline
        .spans
        .iter()
        .flat_map(|span| [span.start, span.end]);
    edges.chain([timeline.to]).filter(|&at| at > now).min()
}

/// The door's state at `unix_seconds`: the state of the span it falls in,
/// and normal outside every span, after the timeline's end, and when the
/// time is not known.
pub(crate) fn mode_at(timeline: &Timeline, unix_seconds: Option<i64>) -> Mode {
    let Some(now) = unix_seconds else {
        return Mode::Normal;
    };
    if now >= timeline.to {
        return Mode::Normal;
    }
    let span = timeline
        .spans
        .iter()
        .find(|span| span.start <= now && now < span.end);
    match span.map(|span| span.state.as_str()) {
        Some("open") => Mode::Open,
        Some("locked") => Mode::Locked,
        _ => Mode::Normal,
    }
}
