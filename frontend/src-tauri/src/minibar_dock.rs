//! The compact bar sticks to the top edge of a screen.
//!
//! Dragged close to the top of the work area, the bar docks: it snaps flush
//! against the edge once the drag settles, and the page draws it as a tab
//! hanging from the edge. A docked bar slides out of the way while the cursor
//! is far from it and comes back as the cursor approaches — the page does the
//! animation; this side only knows the geometry. It tells the page whether the
//! bar is docked and, while it is, how far the cursor is from the window.
//!
//! The cursor has to be polled: once the bar has slid up, the window is mostly
//! transparent and the page gets no pointer events until the cursor is on it,
//! which is too late to start sliding back.

use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex,
};
use std::time::Duration;

use tauri::{Emitter, LogicalSize, PhysicalPosition, Runtime, WebviewWindow, WindowEvent};

/// How close to the top edge (logical px) a drag has to bring the bar to dock.
const SNAP_DISTANCE: f64 = 28.0;
/// A drag has ended when the window has not moved for this long.
const SETTLE_DELAY: Duration = Duration::from_millis(220);
/// How often the cursor is read while the bar is docked.
const POINTER_INTERVAL: Duration = Duration::from_millis(40);

static DOCKED: AtomicBool = AtomicBool::new(false);
/// Bumped on every move; a settle timer only acts if no move came after it.
static MOVE_EPOCH: AtomicU64 = AtomicU64::new(0);
/// Bumped to stop the cursor watch of an earlier dock or an earlier window.
static POINTER_EPOCH: AtomicU64 = AtomicU64::new(0);
/// Where the bar was last put, so it comes back there — docked if it was.
static LAST_POSITION: Mutex<Option<PhysicalPosition<i32>>> = Mutex::new(None);
/// How far the bar was lifted to make room for the notes below it.
static NOTES_LIFT: Mutex<Option<i32>> = Mutex::new(None);

/// Where the bar was last left in this run of the app.
pub fn last_position() -> Option<PhysicalPosition<i32>> {
    *LAST_POSITION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Follow a newly created bar window and work out whether it starts docked.
pub fn attach<R: Runtime>(window: &WebviewWindow<R>) {
    DOCKED.store(false, Ordering::SeqCst);
    POINTER_EPOCH.fetch_add(1, Ordering::SeqCst);
    *NOTES_LIFT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;

    let handle = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Moved(position) = event {
            on_moved(&handle, *position);
        }
    });
    if let Ok(position) = window.outer_position() {
        on_moved(window, position);
    }
}

/// The top of the work area the bar is on, and that area's left and right.
struct Edge {
    top: i32,
    left: i32,
    right: i32,
    bottom: i32,
    scale: f64,
}

fn edge_for<R: Runtime>(
    window: &WebviewWindow<R>,
    position: PhysicalPosition<i32>,
) -> Option<Edge> {
    let size = window.outer_size().ok()?;
    let centre_x = position.x as f64 + size.width as f64 / 2.0;
    // The top edge, not the centre: a bar dragged up belongs to the screen
    // whose edge it is nearing, even while most of it is still on another.
    let probe_y = position.y as f64 + 1.0;
    let monitor = window
        .monitor_from_point(centre_x, probe_y)
        .ok()
        .flatten()
        .or_else(|| window.current_monitor().ok().flatten())?;
    let work = monitor.work_area();
    Some(Edge {
        top: work.position.y,
        left: work.position.x,
        right: work.position.x + work.size.width as i32,
        bottom: work.position.y + work.size.height as i32,
        scale: monitor.scale_factor(),
    })
}

fn on_moved<R: Runtime>(window: &WebviewWindow<R>, position: PhysicalPosition<i32>) {
    *LAST_POSITION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(position);
    let Some(edge) = edge_for(window, position) else {
        return;
    };
    let gap = (position.y - edge.top) as f64 / edge.scale;
    let docked = gap <= SNAP_DISTANCE;
    set_docked(window, docked);
    if docked {
        settle_later(window.clone());
    }
}

/// Once the drag has stopped, pull a docked bar flush against the edge.
fn settle_later<R: Runtime>(window: WebviewWindow<R>) {
    let epoch = MOVE_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SETTLE_DELAY).await;
        if MOVE_EPOCH.load(Ordering::SeqCst) != epoch || !DOCKED.load(Ordering::SeqCst) {
            return;
        }
        let (Ok(position), Ok(size)) = (window.outer_position(), window.outer_size()) else {
            return;
        };
        let Some(edge) = edge_for(&window, position) else {
            return;
        };
        let max_x = (edge.right - size.width as i32).max(edge.left);
        let target = PhysicalPosition::new(position.x.clamp(edge.left, max_x), edge.top);
        if target != position {
            let _ = window.set_position(target);
        }
    });
}

fn set_docked<R: Runtime>(window: &WebviewWindow<R>, docked: bool) {
    if DOCKED.swap(docked, Ordering::SeqCst) == docked {
        return;
    }
    let _ = window.emit_to(window.label(), "minibar-dock", docked);
    if docked {
        watch_pointer(window.clone());
    } else {
        POINTER_EPOCH.fetch_add(1, Ordering::SeqCst);
    }
}

/// While docked: tell the page how far (logical px) the cursor is from the
/// window, 0 when it is over it.
fn watch_pointer<R: Runtime>(window: WebviewWindow<R>) {
    let epoch = POINTER_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        let mut last: Option<f64> = None;
        let mut ticker = tokio::time::interval(POINTER_INTERVAL);
        loop {
            ticker.tick().await;
            if POINTER_EPOCH.load(Ordering::SeqCst) != epoch || !DOCKED.load(Ordering::SeqCst) {
                return;
            }
            // Fails once the window is gone; the watch goes with it.
            let Ok(cursor) = window.cursor_position() else {
                return;
            };
            if !window.is_visible().unwrap_or(false) {
                continue;
            }
            let (Ok(position), Ok(size), Ok(scale)) = (
                window.outer_position(),
                window.outer_size(),
                window.scale_factor(),
            ) else {
                continue;
            };
            let distance = distance_to_rect(
                (cursor.x, cursor.y),
                (position.x as f64, position.y as f64),
                (size.width as f64, size.height as f64),
            ) / scale;
            let distance = distance.round();
            if last != Some(distance) {
                last = Some(distance);
                let _ = window.emit_to(window.label(), "minibar-pointer", distance);
            }
        }
    });
}

/// Distance from a point to a rectangle; 0 inside it.
fn distance_to_rect(point: (f64, f64), origin: (f64, f64), size: (f64, f64)) -> f64 {
    let dx = (origin.0 - point.0)
        .max(point.0 - (origin.0 + size.0))
        .max(0.0);
    let dy = (origin.1 - point.1)
        .max(point.1 - (origin.1 + size.1))
        .max(0.0);
    dx.hypot(dy)
}

/// Whether the bar is docked now, for a page that has just loaded.
#[tauri::command]
pub async fn minibar_dock_state() -> bool {
    DOCKED.load(Ordering::SeqCst)
}

/// Unroll the notes below the bar, or roll them up again.
///
/// The window grows downwards. An undocked bar too close to the bottom of the
/// screen for that is lifted first, and put back when the notes close.
#[tauri::command]
pub async fn set_minibar_notes_open<R: Runtime>(
    window: WebviewWindow<R>,
    open: bool,
) -> Result<(), String> {
    let height = crate::minibar::MINIBAR_HEIGHT
        + if open {
            crate::minibar::MINIBAR_NOTES_HEIGHT
        } else {
            0.0
        };
    window
        .set_size(LogicalSize::new(crate::minibar::MINIBAR_WIDTH, height))
        .map_err(|error| error.to_string())?;

    let mut lift = NOTES_LIFT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if DOCKED.load(Ordering::SeqCst) {
        *lift = None;
        return Ok(());
    }
    let position = window.outer_position().map_err(|error| error.to_string())?;
    if open {
        let size = window.outer_size().map_err(|error| error.to_string())?;
        if let Some(edge) = edge_for(&window, position) {
            let overflow = position.y + size.height as i32 - edge.bottom;
            if overflow > 0 {
                let lifted = overflow.min(position.y - edge.top).max(0);
                *lift = Some(lifted);
                let _ = window.set_position(PhysicalPosition::new(position.x, position.y - lifted));
            }
        }
    } else if let Some(lifted) = lift.take() {
        let _ = window.set_position(PhysicalPosition::new(position.x, position.y + lifted));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::distance_to_rect;

    #[test]
    fn distance_is_zero_inside_and_straight_or_diagonal_outside() {
        let origin = (100.0, 0.0);
        let size = (440.0, 48.0);
        assert_eq!(distance_to_rect((300.0, 20.0), origin, size), 0.0);
        assert_eq!(distance_to_rect((300.0, 148.0), origin, size), 100.0);
        assert_eq!(distance_to_rect((40.0, 20.0), origin, size), 60.0);
        assert_eq!(distance_to_rect((570.0, 88.0), origin, size), 50.0);
    }
}
