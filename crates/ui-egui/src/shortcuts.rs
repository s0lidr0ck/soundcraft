//! Keyboard shortcuts. Most come from the command registry's `shortcut` strings; a few
//! context-sensitive keys (Space, Enter, F-keys, nudge, Tab) are handled here.

use crate::{MainWindow, SoundApp};
use egui::{Key, Modifiers};
use serde_json::json;

/// Parse "Cmd+Shift+X" style strings. Returns None for descriptive ones ("F1-F4", "Plus/Minus").
pub fn parse(s: &str) -> Option<(Modifiers, Key)> {
    let mut m = Modifiers::NONE;
    let mut key = None;
    for part in s.split('+') {
        match part.trim() {
            "Cmd" => m.command = true,
            "Shift" => m.shift = true,
            "Alt" => m.alt = true,
            "Ctrl" => m.ctrl = true,
            "" => {}
            k => {
                if key.is_some() {
                    return None;
                }
                key = Some(match k {
                    "Space" => Key::Space,
                    "Enter" => Key::Enter,
                    "Home" => Key::Home,
                    "End" => Key::End,
                    "Tab" => Key::Tab,
                    "[" => Key::OpenBracket,
                    "]" => Key::CloseBracket,
                    "," => Key::Comma,
                    "." => Key::Period,
                    "/" => Key::Slash,
                    "'" => Key::Quote,
                    "=" => Key::Equals,
                    other => Key::from_name(other)?,
                });
            }
        }
    }
    key.map(|k| (m, k))
}

/// Whether the held modifiers `got` fire a shortcut that asks for `want`.
fn mods_match(want: Modifiers, got: Modifiers, mac: bool) -> bool {
    // Off macOS egui reports Ctrl as both `ctrl` and `command`: Ctrl is the shortcut's Cmd, and a shortcut's
    // Ctrl (the Mac Control key) has no key of its own there, so Ctrl+X is Cut, not Cmd+Ctrl+X.
    let ctrl = if mac { want.ctrl == got.ctrl } else { !want.ctrl };
    want.command == got.command && want.shift == got.shift && want.alt == got.alt && ctrl
}

/// Cmd+Ctrl+S opens Search. Off macOS Ctrl+S is Save (Cmd+S), so Search is reached from the Window menu.
fn search_chord(m: Modifiers, mac: bool) -> bool {
    mac && m.command && m.ctrl
}

/// What a clipboard chord asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clip {
    Cut,
    Copy,
    Paste,
}

/// The clipboard request `key` with `m` makes, if any: Cmd/Ctrl+X/C/V, the dedicated Cut/Copy/Paste
/// keys, and on Windows also Shift+Delete, Ctrl+Insert and Shift+Insert.
fn clipboard_chord(key: Key, m: Modifiers, windows: bool) -> Option<Clip> {
    match key {
        Key::Cut => Some(Clip::Cut),
        Key::Copy => Some(Clip::Copy),
        Key::Paste => Some(Clip::Paste),
        Key::X if m.command_only() => Some(Clip::Cut),
        Key::C if m.command_only() => Some(Clip::Copy),
        Key::V if m.command_only() => Some(Clip::Paste),
        Key::Delete if windows && m.shift => Some(Clip::Cut),
        Key::Insert if windows && m.ctrl => Some(Clip::Copy),
        Key::Insert if windows && m.shift => Some(Clip::Paste),
        _ => None,
    }
}

/// Give this pass's clipboard chords to the widget that wants the keyboard. Call it once per
/// viewport pass, before any widget.
///
/// The native shell turns `clipboard_shortcuts` off, so Cmd/Ctrl+X/C/V arrive as ordinary key
/// presses and [`handle`] runs Edit › Cut, Copy and Paste on clips (#1473): left on, the window
/// layer swallowed them into clipboard events no shortcut reads, and dropped a paste whenever the
/// system clipboard held no text. Text fields listen for those clipboard events, so while one is
/// focused a chord becomes the event again (a paste asks the window for the clipboard's text,
/// which arrives in the next pass). With no field focused a copy is also offered to selected
/// label text. The browser sends its own clipboard events, so the web build does nothing here.
pub fn clipboard_keys(ctx: &egui::Context) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let editing = ctx.egui_wants_keyboard_input();
    let windows = ctx.os() == egui::os::OperatingSystem::Windows;
    let paste = ctx.input_mut(|i| {
        let mut paste = false;
        let mut copy = false;
        for e in &mut i.events {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = e else { continue };
            match clipboard_chord(*key, *modifiers, windows) {
                // Replaced in place: the field must not also see the key (Shift+Delete would
                // delete a character as well), and the other events keep their positions.
                Some(Clip::Cut) if editing => *e = egui::Event::Cut,
                Some(Clip::Copy) if editing => *e = egui::Event::Copy,
                Some(Clip::Paste) if editing => paste = true,
                Some(Clip::Copy) => copy = true,
                _ => {}
            }
        }
        if copy {
            i.events.push(egui::Event::Copy);
        }
        paste
    });
    if paste {
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        ctx.request_repaint();
    }
}

/// Commands a host auto-repeat runs again, so holding a zoom key keeps zooming (#1377). Every other
/// shortcut ignores auto-repeats (#1063): holding Cmd+Z undoes once. Decided by the command a chord
/// resolves to, not by its keys.
const REPEATING: [&str; 3] = ["view.zoom_in", "view.zoom_out", "view.waveform_zoom"];

/// Dispatch global keyboard shortcuts once no text field or dialog wants input, ignoring non-scripted auto-repeats
/// (except the [`REPEATING`] zoom commands) while still running a host press alongside a scripted one.
pub fn handle(app: &mut SoundApp, ctx: &egui::Context) {
    // Cmd+= and Cmd+0 are Mix/Edit and Quantize to Grid, not egui's built-in interface zoom.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    if ctx.egui_wants_keyboard_input() || app.dialogs.open.is_some() || app.ui.show_preferences {
        return;
    }
    // A scripted press is identified by its exact position in this pass's events, not by matching
    // key/modifier values. A fresh host press with the same chord must still run its own action in
    // addition to the scripted one, while a host auto-repeat runs only a zoom command: egui rewrites
    // an event's `repeat` flag from the physical key state, which says nothing about whether that
    // particular event was injected.
    let scripted = app.scripted_key_press_indices();
    let events: Vec<(Key, Modifiers, bool)> = ctx.input(|i| {
        i.events
            .iter()
            .enumerate()
            .filter_map(|(index, e)| match e {
                egui::Event::Key { key, pressed: true, repeat, modifiers, .. } => Some((*key, *modifiers, *repeat && !scripted.contains(&index))),
                _ => None,
            })
            .collect()
    });
    // Cmd and Ctrl are separate keys only on macOS (asked at runtime so the web build in a Mac browser agrees).
    let mac = ctx.os() == egui::os::OperatingSystem::Mac;
    for (key, mods, repeat) in events {
        if repeat {
            if let Some((id, params)) = repeating_command(key, mods, mac) {
                let _ = app.run(id, params);
            }
            continue;
        }
        if fixed(app, key, mods, mac) {
            continue;
        }
        if let Some(id) = command_for(key, mods, mac) {
            if crate::menus::invoke_shortcut_dialog(app, id) {
                continue;
            }
            let _ = app.run(id, json!({}));
        } else if let Some(c) =
            crate::menus::UI_COMMANDS.iter().find(|c| c.3.and_then(parse).is_some_and(|(m, k)| k == key && mods_match(m, mods, mac)))
        {
            let _ = app.run(c.0, json!({}));
        }
    }
    // Tab moves the insertion point here, so egui must not also hand focus to the first widget: that would block every shortcut.
    ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
}

/// The command a host auto-repeat of this chord runs, if it is one of [`REPEATING`]. A repeat
/// resolves its chord the way a press falls through to these zooms ([`zoom_key`], then the
/// registry) without the earlier [`fixed`] / [`fixed_more`] arms, so an arm added there for one of
/// these chords must keep the two paths in agreement.
fn repeating_command(key: Key, mods: Modifiers, mac: bool) -> Option<(&'static str, serde_json::Value)> {
    zoom_key(key, mods, mac).or_else(|| command_for(key, mods, mac).map(|id| (id, json!({})))).filter(|(id, _)| REPEATING.contains(id))
}

fn command_for(key: Key, mods: Modifiers, mac: bool) -> Option<&'static str> {
    soundcraft_engine::command_specs()
        .iter()
        .find(|c| c.shortcut.and_then(parse).is_some_and(|(m, k)| k == key && mods_match(m, mods, mac)))
        .map(|c| c.id)
}

/// Keys with context-dependent meaning. Returns true when handled.
fn fixed(app: &mut SoundApp, key: Key, m: Modifiers, mac: bool) -> bool {
    let plain = !m.command && !m.alt && !m.ctrl && !m.shift;
    match key {
        Key::Space if !m.command && !m.alt => {
            let id = if m.shift { "transport.half_speed" } else { "transport.toggle" };
            let _ = app.run(id, json!({}));
            true
        }
        Key::Space if m.command => {
            let _ = app.run("transport.record", json!({}));
            true
        }
        Key::S if search_chord(m, mac) => {
            let _ = app.run("window.search", json!({}));
            true
        }
        Key::Equals if m.command && !(mac && m.ctrl) => {
            let _ = app.run("window.toggle_mix_edit", json!({}));
            true
        }
        Key::Enter if plain => {
            let _ = app.run("markers.add", json!({}));
            true
        }
        // Shift+=/- step the Grid value and Alt+Shift+=/- the Nudge value; macOS sends Alt+Shift+= as =, not +.
        Key::Plus | Key::Equals | Key::Minus if m.shift && !m.command && !m.ctrl && app.ui.window == MainWindow::Edit => {
            let step = if key == Key::Minus { -1 } else { 1 };
            let _ = app.run(if m.alt { "view.nudge" } else { "view.grid" }, json!({"step": step}));
            true
        }
        Key::F12 if plain => {
            let _ = app.run("transport.record", json!({}));
            true
        }
        Key::F1 | Key::F2 | Key::F3 | Key::F4 if plain => {
            edit_mode_key(app, key);
            true
        }
        Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 if m.alt && !m.command && !m.ctrl && !m.shift => {
            let f = match key {
                Key::Num1 => Key::F1,
                Key::Num2 => Key::F2,
                Key::Num3 => Key::F3,
                _ => Key::F4,
            };
            edit_mode_key(app, f);
            true
        }
        Key::Backtick if plain => {
            cycle_edit_mode(app);
            true
        }
        Key::F5 | Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 if plain => {
            let _ = app.run("edit.tool", json!({"tool": tool_key(app, key)}));
            true
        }
        // Cmd+6 / Cmd+7 (Ctrl off macOS) are Pencil and the Smart Tool. Cmd+1–5 keep their window and Session
        // keys: egui reports the numeric keypad's digits as the main keyboard's, so the two can't be told apart.
        Key::Num6 | Key::Num7 if mods_match(Modifiers::COMMAND, m, mac) => {
            let tool = if key == Key::Num6 { "pencil" } else { "smart" };
            let _ = app.run("edit.tool", json!({"tool": tool}));
            true
        }
        _ => fixed_more(app, key, m, mac),
    }
}

/// The rest of [`fixed`]'s keys.
fn fixed_more(app: &mut SoundApp, key: Key, m: Modifiers, mac: bool) -> bool {
    let plain = !m.command && !m.alt && !m.ctrl && !m.shift;
    match key {
        // `=` is what a bare press of the unlabeled +/- key reports (it has no shift held, so it
        // can never be `Key::Plus`); `Key::Plus` is kept for a keyboard that does send it (e.g. a
        // numpad +). `!m.alt` leaves Alt+= to `window.event_list`'s registered shortcut.
        Key::Plus | Key::Equals | Key::Minus if !m.command && !m.alt && app.ui.window == MainWindow::Edit => {
            let dir = if key == Key::Minus { -1 } else { 1 };
            let _ = app.run("edit.nudge", json!({"direction": dir}));
            true
        }
        // Alt+Tab goes back (off macOS the desktop takes Alt+Tab, so Ctrl+Tab does) and Shift extends the selection.
        // Cmd+Alt+Tab is Tab to Transient, and the Mac's Control+Tab is Pro Tools' Go to Next Clip.
        Key::Tab if !(m.command && (mac || m.alt)) && !(mac && m.ctrl) => {
            let _ = app.run("edit.tab", json!({"back": m.alt || (!mac && m.ctrl), "extend": m.shift}));
            true
        }
        // The registry's key families "Cmd+Alt+0-5" and "Cmd+Ctrl+1-5", which `parse` can't read ("Cmd+Alt+[ / ]" is in `zoom_key`).
        Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 if mods_match(Modifiers::COMMAND | Modifiers::ALT, m, mac) => {
            let _ = app.run("clip.rating", json!({"rating": digit(key)}));
            true
        }
        Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 if mods_match(Modifiers::COMMAND | Modifiers::CTRL, m, mac) => {
            let _ = app.run("view.zoom_preset", json!({"preset": digit(key)}));
            true
        }
        // Pro Tools' Control+Up/Down track height, on Ctrl off macOS too (its Start key isn't available there).
        Key::ArrowUp | Key::ArrowDown if m.ctrl && (m.alt || !m.shift) => {
            let s = app.engine.session();
            let tracks = crate::edit_window::height_targets(s, m, &s.edit.selected_tracks);
            let id = if key == Key::ArrowUp { "track.height_up" } else { "track.height_down" };
            let _ = app.run(id, if tracks.is_empty() { json!({}) } else { json!({"tracks": tracks}) });
            true
        }
        // Extend Edit Down is Control+Shift+;, which macOS sends as Control+:.
        Key::Semicolon | Key::Colon if mac && m.ctrl && m.shift && !m.alt && !m.command => {
            let _ = app.run("edit.extend_selection_down", json!({}));
            true
        }
        // Ctrl+Shift+Up/Down nudge clip gain off macOS too (the registry's Ctrl is the Mac Control key).
        Key::ArrowUp | Key::ArrowDown if !mac && m.ctrl && m.shift && !m.alt => {
            let id = if key == Key::ArrowUp { "clip.gain_nudge_up" } else { "clip.gain_nudge_down" };
            let _ = app.run(id, json!({}));
            true
        }
        // With an edit selection (a selected clip makes one), Left/Right centre the Edit window on its start/end.
        // Over the piano roll the arrows move its notes instead.
        Key::ArrowLeft | Key::ArrowRight if plain && app.ui.window == MainWindow::Edit && !piano_roll_has_pointer(app) => {
            centre_on_selection(app, key == Key::ArrowRight)
        }
        // Commands Keyboard Focus: single-key editing (Edit window only).
        k if !m.command
            && !m.alt
            && !m.ctrl
            && app.ui.window == MainWindow::Edit
            && app.engine.session().edit.keyboard_focus == "commands"
            && commands_focus(app, k, m.shift) =>
        {
            true
        }
        _ => match zoom_key(key, m, mac) {
            Some((id, params)) => {
                let _ = app.run(id, params);
                true
            }
            None => false,
        },
    }
}

/// The zoom keys [`fixed`] handles, shared with [`repeating_command`]: T / R zoom the timeline in / out
/// (Commands Focus style) and the registry's "Cmd+Alt+[ / ]", which `parse` can't read, steps the waveform zoom.
fn zoom_key(key: Key, m: Modifiers, mac: bool) -> Option<(&'static str, serde_json::Value)> {
    let plain = !m.command && !m.alt && !m.ctrl && !m.shift;
    match key {
        Key::R | Key::T if plain => Some((if key == Key::T { "view.zoom_in" } else { "view.zoom_out" }, json!({}))),
        Key::OpenBracket | Key::CloseBracket if mods_match(Modifiers::COMMAND | Modifiers::ALT, m, mac) => {
            Some(("view.waveform_zoom", json!({"factor": if key == Key::CloseBracket { 2.0 } else { 0.5 }})))
        }
        _ => None,
    }
}

/// F1–F4 (and Alt+1–4) choose Shuffle, Slip, Spot and Grid; F4 in Grid switches between Absolute and
/// Relative unless Edit/Tool Mode Keyboard Lock is on. Holding F4 with F1–F3, in either order, adds Snap to Grid.
fn edit_mode_key(app: &mut SoundApp, pressed: Key) {
    let ctx = app.docking_context.clone();
    let held = |k: Key| ctx.as_ref().is_some_and(|c| c.input(|i| i.key_down(k)));
    let (key, snap) = match [Key::F1, Key::F2, Key::F3].into_iter().find(|k| *k != pressed && held(*k)) {
        Some(k) if pressed == Key::F4 => (k, true),
        _ => (pressed, pressed != Key::F4 && held(Key::F4)),
    };
    let ed = &app.engine.session().edit;
    let keep_grid = ed.edit_mode.is_grid() && ed.flag("options.edit_tool_mode_keyboard_lock");
    // F4 pressed first in an F4+F1–F3 chord switched the Grid mode on its own; put that back.
    let flipped = egui::Id::unique("edit_mode_key_grid_before_f4");
    let before =
        (ed.edit_mode.is_grid() && !keep_grid).then(|| if ed.flag(soundcraft_engine::edit::RELATIVE_GRID) { "relative_grid" } else { "grid" });
    let restore = match &ctx {
        Some(c) if pressed == Key::F4 => {
            c.data_mut(|d| d.insert_temp(flipped, before));
            None
        }
        Some(c) if snap => c.data_mut(|d| d.remove_temp::<Option<&'static str>>(flipped)).flatten(),
        _ => None,
    };
    let mode = match key {
        Key::F1 => "shuffle",
        Key::F2 => "slip",
        Key::F3 => "spot",
        _ if keep_grid => return,
        _ => grid_mode(app),
    };
    if let Some(grid) = restore {
        let _ = app.run("edit.mode", json!({"mode": grid}));
    }
    let _ = app.run("edit.mode", json!({"mode": mode, "snap_to_grid": snap}));
}

/// F5–F10 choose the Edit tools; F7 with F6 or F8 held, in either order, chooses the Smart Tool.
fn tool_key(app: &SoundApp, pressed: Key) -> &'static str {
    let ctx = app.docking_context.as_ref();
    let held = |k: Key| ctx.is_some_and(|c| c.input(|i| i.key_down(k)));
    match pressed {
        Key::F6 | Key::F8 if held(Key::F7) => "smart",
        Key::F7 if held(Key::F6) || held(Key::F8) => "smart",
        Key::F5 => "zoom",
        Key::F6 => "trim",
        Key::F7 => "selector",
        Key::F8 => "grabber",
        Key::F9 => "scrubber",
        _ => "pencil",
    }
}

/// What choosing Grid picks: in Grid the other Grid mode, from another mode the one used last.
pub(crate) fn grid_mode(app: &SoundApp) -> &'static str {
    use soundcraft_model::EditMode;
    let ed = &app.engine.session().edit;
    match ed.edit_mode {
        EditMode::Grid => "relative_grid",
        EditMode::GridRelative => "grid",
        _ if ed.flag(soundcraft_engine::edit::RELATIVE_GRID) => "relative_grid",
        _ => "grid",
    }
}

/// ` steps through the Edit modes in F-key order, past a locked Shuffle.
fn cycle_edit_mode(app: &mut SoundApp) {
    use soundcraft_model::EditMode;
    let ed = &app.engine.session().edit;
    let mode = match ed.edit_mode {
        EditMode::Shuffle => "slip",
        EditMode::Slip => "spot",
        EditMode::Spot => grid_mode(app),
        _ if ed.flag(soundcraft_engine::edit::SHUFFLE_LOCK) => "slip",
        _ => "shuffle",
    };
    let _ = app.run("edit.mode", json!({"mode": mode}));
}

/// Single-key commands. Returns true when `key` was handled.
fn commands_focus(app: &mut SoundApp, key: Key, shift: bool) -> bool {
    let id = match key {
        Key::Z if shift => "edit.redo",
        Key::P if shift => "edit.extend_selection_up",
        Key::Semicolon | Key::Colon if shift => "edit.extend_selection_down",
        _ if shift => return false,
        Key::A => "edit.trim_start_to_insertion",
        Key::S => "edit.trim_end_to_insertion",
        Key::D => "edit.fade_to_start",
        Key::G => "edit.fade_to_end",
        Key::F => "edit.fades_create",
        Key::Comma | Key::Period | Key::M | Key::Slash => {
            let direction = if matches!(key, Key::Period | Key::Slash) { 1 } else { -1 };
            let _ = app.run("edit.nudge", json!({"direction": direction, "next": matches!(key, Key::M | Key::Slash)}));
            return true;
        }
        Key::Num0 => "edit.timeline_to_selection",
        Key::O => "edit.selection_to_timeline",
        Key::OpenBracket => "transport.play_selection",
        Key::CloseBracket => "edit.play_timeline",
        Key::B => "edit.separate",
        Key::X => "edit.cut",
        Key::C => "edit.copy",
        Key::V => "edit.paste",
        Key::Z => "edit.undo",
        Key::N => "options.insertion_follows_playback",
        Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 => {
            let _ = app.run("view.zoom_preset", json!({"preset": digit(key)}));
            return true;
        }
        Key::P => {
            move_selection_track(app, -1);
            return true;
        }
        Key::Semicolon => {
            move_selection_track(app, 1);
            return true;
        }
        Key::L | Key::Quote => {
            tab_clip(app, key == Key::L);
            return true;
        }
        Key::E => {
            crate::edit_window::zoom_toggle(app);
            return true;
        }
        _ => return false,
    };
    let _ = app.run(id, json!({}));
    true
}

/// Move the edit selection to the track above/below (P / ;).
fn move_selection_track(app: &mut SoundApp, dir: i64) {
    let s = app.engine.session();
    let visible: Vec<u64> = s.tracks.iter().filter(|t| !t.hidden).map(|t| t.id.0).collect();
    let Some(cur) = s.edit.selected_tracks.first().map(|t| t.0) else { return };
    let Some(i) = visible.iter().position(|t| *t == cur) else { return };
    let j = (i as i64 + dir).clamp(0, visible.len() as i64 - 1) as usize;
    if let Some(t) = visible.get(j) {
        let sel = s.edit.selection;
        let _ = app.run("edit.select", json!({"tracks": [t], "start": sel.start, "end": sel.end, "exact": true}));
    }
}

/// Tab to the previous/next clip boundary on the selected tracks (L / ').
fn tab_clip(app: &mut SoundApp, back: bool) {
    let s = app.engine.session();
    let at = s.edit.selection.start;
    let mut edges: Vec<i64> =
        s.tracks.iter().filter(|t| s.edit.selected_tracks.contains(&t.id)).flat_map(|t| t.clips().iter().flat_map(|c| [c.start, c.end()])).collect();
    edges.sort_unstable();
    let next = if back { edges.iter().rev().find(|e| **e < at).copied() } else { edges.iter().find(|e| **e > at).copied() };
    if let Some(n) = next {
        let _ = app.run("transport.locate", json!({"at": n}));
    }
}

/// Scroll the Edit window so the edit selection's start (or `end`) is in its middle. Returns false, leaving
/// the key unhandled, when there is only an insertion point.
fn centre_on_selection(app: &mut SoundApp, end: bool) -> bool {
    let s = app.engine.session();
    let sel = s.edit.selection;
    if sel.start >= sel.end {
        return false;
    }
    let [x0, _, x1, _] = app.edit_layout.timeline;
    let half = f64::from((x1 - x0).max(0.0)) / 2.0 * s.edit.zoom.samples_per_px.max(0.01);
    let at = if end { sel.end } else { sel.start };
    let _ = app.run("view.scroll", json!({"to": at.saturating_sub(soundcraft_time::to_samples(half)).max(0)}));
    true
}

/// Whether the pointer is over the MIDI editor, whose piano roll moves the selected notes with the arrow keys.
fn piano_roll_has_pointer(app: &SoundApp) -> bool {
    app.ui.show_midi_editor
        && app.docking_context.as_ref().is_some_and(|c| {
            let body = c.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("soundcraft-midi-body")));
            c.pointer_hover_pos().zip(body).is_some_and(|(p, r)| r.contains(p))
        })
}

fn digit(key: Key) -> u8 {
    key.name().parse().unwrap_or(0)
}

/// Shortcut text for display. The registry stores macOS-style strings ("Cmd+N"); off macOS the Cmd
/// modifier is the Ctrl key. Mac-only Control chords ("Cmd+Ctrl+S") have no key there, so they show nothing.
pub fn shortcut_label(s: &str, mac: bool) -> String {
    let has = |m: &str| s.split('+').any(|p| p == m);
    if mac || (!has("Cmd") && !has("Ctrl")) || PC_CTRL_CHORDS.contains(&s) {
        return s.to_string();
    }
    if has("Ctrl") {
        return String::new();
    }
    s.split('+').map(|p| if p == "Cmd" { "Ctrl" } else { p }).collect::<Vec<_>>().join("+")
}

/// Mac Control chords that `fixed` also maps to Ctrl off macOS.
const PC_CTRL_CHORDS: [&str; 4] = ["Ctrl+Up", "Ctrl+Down", "Ctrl+Shift+Up", "Ctrl+Shift+Down"];

#[cfg(test)]
mod label_tests {
    use super::shortcut_label;

    #[test]
    fn cmd_becomes_ctrl_off_mac() {
        assert_eq!(shortcut_label("Cmd+N", false), "Ctrl+N");
        assert_eq!(shortcut_label("Cmd+Shift+W", false), "Ctrl+Shift+W");
        assert_eq!(shortcut_label("Cmd+Alt+B", false), "Ctrl+Alt+B");
        assert_eq!(shortcut_label("Cmd+,", false), "Ctrl+,");
        assert_eq!(shortcut_label("Shift+R", false), "Shift+R");
        assert_eq!(shortcut_label("Cmd+Ctrl+S", false), "");
        assert_eq!(shortcut_label("", false), "");
        assert_eq!(shortcut_label("Ctrl+Up", false), "Ctrl+Up");
        assert_eq!(shortcut_label("Ctrl+Shift+Down", false), "Ctrl+Shift+Down");
        assert_eq!(shortcut_label("Ctrl+Shift+P", false), "");
        assert_eq!(shortcut_label("Ctrl+Alt+L", false), "");
    }

    #[test]
    fn unchanged_on_mac() {
        for s in ["Cmd+N", "Cmd+Shift+W", "Cmd+Alt+B", "Shift+R", "Cmd+Ctrl+S"] {
            assert_eq!(shortcut_label(s, true), s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Clip, clipboard_chord, clipboard_keys, command_for, mods_match, parse, search_chord};
    use egui::{Key, Modifiers};

    /// Ctrl held on Windows or Linux, as egui reports it.
    const PC_CTRL: Modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    /// Cmd held on macOS.
    const MAC_CMD: Modifiers = Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE };

    fn want(s: &str) -> Modifiers {
        parse(s).map(|(m, _)| m).unwrap()
    }

    #[test]
    fn clipboard_chords_are_the_platform_ones() {
        for (key, clip) in [(Key::X, Clip::Cut), (Key::C, Clip::Copy), (Key::V, Clip::Paste)] {
            assert_eq!(clipboard_chord(key, PC_CTRL, true), Some(clip));
            assert_eq!(clipboard_chord(key, MAC_CMD, false), Some(clip));
            assert_eq!(clipboard_chord(key, Modifiers::NONE, true), None, "a bare letter is a single-key command");
            assert_eq!(clipboard_chord(key, Modifiers { shift: true, ..PC_CTRL }, true), None, "Cmd+Shift+V is Paste Special");
        }
        let shift = Modifiers { shift: true, ..Modifiers::NONE };
        assert_eq!(clipboard_chord(Key::Delete, shift, true), Some(Clip::Cut));
        assert_eq!(clipboard_chord(Key::Insert, PC_CTRL, true), Some(Clip::Copy));
        assert_eq!(clipboard_chord(Key::Insert, shift, true), Some(Clip::Paste));
        assert_eq!(clipboard_chord(Key::Delete, shift, false), None, "Shift+Delete is a Windows chord");
        assert_eq!(clipboard_chord(Key::Insert, shift, false), None);
        assert_eq!(clipboard_chord(Key::Copy, Modifiers::NONE, false), Some(Clip::Copy));
    }

    /// One pass over a focused single-line field holding `text`, with `events` as host input.
    fn field_pass(ctx: &egui::Context, text: &mut String, events: Vec<egui::Event>) -> Vec<egui::Event> {
        let mut seen = Vec::new();
        let input = egui::RawInput { events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            clipboard_keys(ui.ctx());
            seen = ui.ctx().input(|i| i.events.clone());
            ui.add(egui::TextEdit::singleline(text)).request_focus();
        });
        out.textures_delta.clear();
        seen
    }

    fn press(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn a_focused_text_field_gets_clipboard_chords_as_clipboard_events() {
        let ctx = egui::Context::default();
        let mut text = String::from("tempo");
        field_pass(&ctx, &mut text, Vec::new());
        field_pass(&ctx, &mut text, Vec::new());
        assert!(ctx.egui_wants_keyboard_input(), "the field is editing");
        field_pass(&ctx, &mut text, vec![press(Key::A, PC_CTRL)]);
        // Copy leaves the text; cut removes the selection. Both keys reach the field as events.
        let seen = field_pass(&ctx, &mut text, vec![press(Key::C, PC_CTRL)]);
        assert!(matches!(seen.as_slice(), [egui::Event::Copy]), "{seen:?}");
        assert_eq!(text, "tempo");
        let seen = field_pass(&ctx, &mut text, vec![press(Key::X, PC_CTRL)]);
        assert!(matches!(seen.as_slice(), [egui::Event::Cut]), "{seen:?}");
        assert_eq!(text, "", "Ctrl+X cut the selected text");
        // A paste keeps its key (the text comes from the window in the next pass).
        let seen = field_pass(&ctx, &mut text, vec![press(Key::V, PC_CTRL)]);
        assert!(matches!(seen.as_slice(), [egui::Event::Key { key: Key::V, .. }]), "{seen:?}");
    }

    #[test]
    fn without_a_text_field_clipboard_chords_stay_keys_for_the_clip_commands() {
        let ctx = egui::Context::default();
        let pass = |events: Vec<egui::Event>| {
            let mut seen = Vec::new();
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                clipboard_keys(ui.ctx());
                seen = ui.ctx().input(|i| i.events.clone());
            });
            out.textures_delta.clear();
            seen
        };
        for key in [Key::X, Key::V] {
            let seen = pass(vec![press(key, PC_CTRL)]);
            assert!(matches!(seen.as_slice(), [egui::Event::Key { key: k, .. }] if *k == key), "{seen:?}");
            assert_eq!(command_for(key, PC_CTRL, false), Some(if key == Key::X { "edit.cut" } else { "edit.paste" }));
        }
        // Copy keeps its key for Edit › Copy and is also offered to selected label text.
        let seen = pass(vec![press(Key::C, PC_CTRL)]);
        assert!(matches!(seen.as_slice(), [egui::Event::Key { key: Key::C, .. }, egui::Event::Copy]), "{seen:?}");
        assert_eq!(command_for(Key::C, PC_CTRL, false), Some("edit.copy"));
    }

    #[test]
    fn ctrl_is_cmd_off_macos() {
        assert!(mods_match(want("Cmd+S"), PC_CTRL, false));
        assert!(mods_match(want("Cmd+Shift+N"), Modifiers { shift: true, ..PC_CTRL }, false));
        assert!(mods_match(want("Cmd+X"), PC_CTRL, false));
        assert!(!mods_match(want("Cmd+Ctrl+X"), PC_CTRL, false));
        assert!(!mods_match(want("Cmd+S"), Modifiers::NONE, false));
        assert!(!search_chord(PC_CTRL, false));
    }

    #[test]
    fn cmd_and_ctrl_stay_apart_on_macos() {
        assert!(mods_match(want("Cmd+S"), MAC_CMD, true));
        assert!(!mods_match(want("Cmd+S"), Modifiers::CTRL, true));
        assert!(mods_match(want("Cmd+Ctrl+X"), Modifiers { ctrl: true, ..MAC_CMD }, true));
        assert!(!mods_match(want("Cmd+X"), Modifiers { ctrl: true, ..MAC_CMD }, true), "Cmd+Ctrl+X is not Cut");
        assert!(!search_chord(MAC_CMD, true));
        assert!(search_chord(Modifiers { ctrl: true, ..MAC_CMD }, true));
    }

    #[test]
    fn shift_cmd_k_prompts_for_a_folder_to_export_clips() {
        assert_eq!(command_for(Key::K, Modifiers { shift: true, ..MAC_CMD }, true), Some("file.export_clips"));
        assert_eq!(command_for(Key::K, Modifiers { shift: true, ..PC_CTRL }, false), Some("file.export_clips"));
        assert!(crate::dialogs::takes_path("file.export_clips"));
    }

    fn app_with_selection() -> crate::SoundApp {
        let mut app = crate::SoundApp::new(soundcraft_engine::demo::demo_engine(), None, crate::Services::default());
        app.ui.window = crate::MainWindow::Edit;
        app.edit_layout.timeline = [0.0, 0.0, 100.0, 100.0];
        app.run("view.zoom_set", serde_json::json!({"samples_per_px": 10.0})).unwrap();
        app.run("edit.select", serde_json::json!({"start": 2000, "end": 4000})).unwrap();
        app
    }

    #[test]
    fn plain_arrows_centre_selection_but_modified_arrows_do_not() {
        for mac in [false, true] {
            for (key, expected) in [(Key::ArrowLeft, 1500), (Key::ArrowRight, 3500)] {
                let mut app = app_with_selection();
                assert!(super::fixed_more(&mut app, key, Modifiers::NONE, mac));
                assert_eq!(app.engine.session().edit.zoom.scroll, expected);
                for modifiers in [Modifiers::SHIFT, Modifiers::ALT, Modifiers::CTRL, Modifiers::COMMAND, PC_CTRL, MAC_CMD] {
                    app.run("view.scroll", serde_json::json!({"to": 123})).unwrap();
                    assert!(!super::fixed_more(&mut app, key, modifiers, mac));
                    assert_eq!(app.engine.session().edit.zoom.scroll, 123, "{key:?} {modifiers:?}");
                }
            }
        }
    }

    #[test]
    fn selection_arrows_do_not_scroll_the_mix_window_or_an_insertion_point() {
        for key in [Key::ArrowLeft, Key::ArrowRight] {
            let mut app = app_with_selection();
            app.ui.window = crate::MainWindow::Mix;
            assert!(!super::fixed_more(&mut app, key, Modifiers::NONE, false));
            assert_eq!(app.engine.session().edit.zoom.scroll, 0);
            app.ui.window = crate::MainWindow::Edit;
            app.run("edit.select", serde_json::json!({"start": 2000, "end": 2000})).unwrap();
            assert!(!super::fixed_more(&mut app, key, Modifiers::NONE, false));
            assert_eq!(app.engine.session().edit.zoom.scroll, 0);
        }
    }

    #[test]
    fn selection_arrows_leave_the_piano_roll_in_control_under_its_pointer() {
        let ctx = egui::Context::default();
        let mut app = app_with_selection();
        app.ui.show_midi_editor = true;
        app.docking_context = Some(ctx.clone());
        let body = egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(100.0, 100.0));
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("soundcraft-midi-body"), body));
        for (pointer, handled) in [(body.center(), false), (egui::pos2(200.0, 200.0), true)] {
            let mut output = ctx.run_ui(egui::RawInput { events: vec![egui::Event::PointerMoved(pointer)], ..Default::default() }, |_| {
                for key in [Key::ArrowLeft, Key::ArrowRight] {
                    app.run("view.scroll", serde_json::json!({"to": 0})).unwrap();
                    assert_eq!(super::fixed_more(&mut app, key, Modifiers::NONE, false), handled);
                    assert_eq!(app.engine.session().edit.zoom.scroll > 0, handled);
                }
            });
            output.textures_delta.clear();
        }
    }
}
