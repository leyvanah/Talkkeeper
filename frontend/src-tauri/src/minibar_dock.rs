//! The compact bar sticks to the top edge of a screen.
//!
//! Dragged close to the top of the work area, the bar docks: it is pulled onto
//! the edge while still being dragged, and the page draws it as a tab hanging
//! from the edge. Docked, it runs along the edge like on a rail — a drag moves
//! it sideways only — until it is pulled well down, when it tears off and
//! follows the cursor again. On Windows that happens in WM_MOVING, where the
//! position a drag proposes can be changed before the window gets there;
//! elsewhere the bar is put against the edge once the drag has settled.
//!
//! A docked bar slides out of the way while the cursor
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

/// How close to the top edge (logical px) a drag has to bring the bar for the
/// edge to pull it in — about a bar's height, so the pull is felt coming.
const SNAP_DISTANCE: f64 = 48.0;
/// How far below the edge (logical px) a docked bar has to be pulled to come
/// off it. Further than the pull-in distance, so it does not flicker between
/// the two right at the line.
const TEAR_OFF_DISTANCE: f64 = 90.0;
/// A drag has ended when the window has not moved for this long.
const SETTLE_DELAY: Duration = Duration::from_millis(220);
/// How often the cursor is read while the bar is docked.
const POINTER_INTERVAL: Duration = Duration::from_millis(40);

static DOCKED: AtomicBool = AtomicBool::new(false);
/// The scale of the screen the bar is on, as f64 bits; read while dragging.
static SCALE_BITS: AtomicU64 = AtomicU64::new(0x3FF0_0000_0000_0000); // 1.0
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
    #[cfg(windows)]
    rails::install(window);
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
    SCALE_BITS.store(edge.scale.to_bits(), Ordering::Relaxed);
    let gap = (position.y - edge.top) as f64 / edge.scale;
    // With the rail in place a bar is either on the edge or more than the
    // pull-in distance from it, so this agrees with the rail's own state.
    let docked = gap <= SNAP_DISTANCE;
    set_docked(window, docked);
    // A docked bar pulled down gives a little before it tears off. The page
    // peels the tab's corners off the edge as daylight opens up under it.
    let _ = window.emit_to(window.label(), "minibar-edge-gap", gap.max(0.0).round());
    if docked {
        settle_later(window.clone());
    }
}

/// Once the drag has stopped, pull a docked bar flush against the edge.
fn settle_later<R: Runtime>(window: WebviewWindow<R>) {
    let epoch = MOVE_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SETTLE_DELAY).await;
        // Still held: a bar resting where it gave under the pull is not done
        // moving. Letting go puts it back (WM_EXITSIZEMOVE in `rails`).
        #[cfg(windows)]
        if rails::dragging() {
            return;
        }
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

/// How much of the pull a docked bar gives before it tears off: it comes away
/// from the edge a little, against resistance, the way a magnet does.
const PULL_GIVE: f64 = 0.3;

/// Where the bar's top edge goes while dragged. `free_top` is where it would
/// be if it simply followed the cursor; the work area's top is at `edge_top`.
///
/// Docked, the bar gives a little as it is pulled down and tears off once the
/// cursor has gone past the tear-off distance, jumping to where the cursor
/// holds it. Free, it is pulled onto the edge within the pull-in distance.
fn rail(free_top: i32, edge_top: i32, scale: f64, docked: bool) -> i32 {
    let pull = (free_top - edge_top) as f64 / scale;
    if docked {
        if pull > TEAR_OFF_DISTANCE {
            free_top
        } else {
            edge_top + (pull.max(0.0) * PULL_GIVE * scale).round() as i32
        }
    } else if pull <= SNAP_DISTANCE {
        edge_top
    } else {
        free_top
    }
}

/// The rail itself: the drag's proposed rectangle, corrected in WM_MOVING.
///
/// The rectangle Windows proposes is the window's current one moved by the
/// mouse's latest step, so once the rail has held the bar on the edge, the
/// pull so far is forgotten — only a jerk larger than the whole tear-off
/// distance in a single step would get it off. The pull is therefore measured
/// from the cursor itself, against the point where the bar was grabbed.
#[cfg(windows)]
mod rails {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

    use tauri::{Runtime, WebviewWindow};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
        WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_MOVING, WM_NCDESTROY,
    };

    const SUBCLASS_ID: usize = 0x7a11;

    /// How far below the window's top the cursor took hold of it.
    static GRAB_Y: AtomicI32 = AtomicI32::new(0);
    static GRABBED: AtomicBool = AtomicBool::new(false);

    /// The bar is being dragged right now.
    pub fn dragging() -> bool {
        GRABBED.load(Ordering::Relaxed)
    }

    pub fn install<R: Runtime>(window: &WebviewWindow<R>) {
        let Ok(hwnd) = window.hwnd() else {
            return;
        };
        let hwnd = hwnd.0 as isize;
        // A window is subclassed on the thread that owns it.
        let _ = window.run_on_main_thread(move || unsafe {
            SetWindowSubclass(hwnd as HWND, Some(procedure), SUBCLASS_ID, 0);
        });
    }

    unsafe extern "system" fn procedure(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        match message {
            WM_ENTERSIZEMOVE => {
                let mut cursor = POINT { x: 0, y: 0 };
                let mut window: RECT = std::mem::zeroed();
                if GetCursorPos(&mut cursor) != 0 && GetWindowRect(hwnd, &mut window) != 0 {
                    GRAB_Y.store(cursor.y - window.top, Ordering::Relaxed);
                    GRABBED.store(true, Ordering::Relaxed);
                }
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            WM_MOVING if lparam != 0 => {
                let rect = &mut *(lparam as *mut RECT);
                if let Some(edge_top) = work_area_top(rect) {
                    let mut cursor = POINT { x: 0, y: 0 };
                    let free_top =
                        if GRABBED.load(Ordering::Relaxed) && GetCursorPos(&mut cursor) != 0 {
                            cursor.y - GRAB_Y.load(Ordering::Relaxed)
                        } else {
                            rect.top
                        };
                    let scale = f64::from_bits(super::SCALE_BITS.load(Ordering::Relaxed));
                    let docked = super::DOCKED.load(Ordering::SeqCst);
                    let top = super::rail(free_top, edge_top, scale.max(0.5), docked);
                    let height = rect.bottom - rect.top;
                    rect.top = top;
                    rect.bottom = top + height;
                }
                1
            }
            WM_EXITSIZEMOVE => {
                GRABBED.store(false, Ordering::Relaxed);
                // Let go before tearing off: the bar springs back onto the edge.
                let mut window: RECT = std::mem::zeroed();
                if super::DOCKED.load(Ordering::SeqCst) && GetWindowRect(hwnd, &mut window) != 0 {
                    if let Some(edge_top) = work_area_top(&window) {
                        if window.top != edge_top {
                            SetWindowPos(
                                hwnd,
                                std::ptr::null_mut(),
                                window.left,
                                edge_top,
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                            );
                        }
                    }
                }
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID);
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, message, wparam, lparam),
        }
    }

    /// The top of the work area of the screen the rectangle is mostly on.
    unsafe fn work_area_top(rect: &RECT) -> Option<i32> {
        let monitor = MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() {
            return None;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        (GetMonitorInfoW(monitor, &mut info) != 0).then_some(info.rcWork.top)
    }
}

#[cfg(test)]
mod tests {
    use super::{distance_to_rect, rail, PULL_GIVE, SNAP_DISTANCE, TEAR_OFF_DISTANCE};

    #[test]
    fn a_free_bar_is_pulled_onto_the_edge_and_a_docked_one_only_torn_off() {
        let top = 0;
        // Free: pulled in within a bar's height, left alone further down.
        assert_eq!(rail(40, top, 1.0, false), top);
        assert_eq!(rail(60, top, 1.0, false), 60);
        // Docked: a pull down gives a little, against resistance…
        assert_eq!(rail(0, top, 1.0, true), top);
        assert_eq!(rail(-30, top, 1.0, true), top);
        assert_eq!(rail(60, top, 1.0, true), 18);
        assert_eq!(rail(90, top, 1.0, true), 27);
        // …until it tears off and follows the cursor.
        assert_eq!(rail(91, top, 1.0, true), 91);
        // Distances are logical: at 150% the same pull is half as many pixels again.
        assert_eq!(rail(120, top, 1.5, true), 36);
        assert_eq!(rail(136, top, 1.5, true), 136);
        // A screen whose work area starts lower, under a top taskbar.
        assert_eq!(rail(70, 40, 1.0, false), 40);
        assert_eq!(rail(100, 40, 1.0, true), 58);
    }

    #[test]
    fn a_bar_given_way_still_counts_as_docked() {
        // The most a docked bar gives must stay inside the pull-in distance,
        // or the window would read as torn off before it is.
        assert!(TEAR_OFF_DISTANCE * PULL_GIVE < SNAP_DISTANCE);
    }

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
