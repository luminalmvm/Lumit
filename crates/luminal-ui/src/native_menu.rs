//! Native macOS menu bar (system bar at the top of the screen), via muda.
//!
//! In plain terms: on a Mac, application menus belong in the bar at the very
//! top of the screen, not inside the window. This module builds that menu and
//! translates clicks/shortcuts into the same actions the in-window menu bar
//! performs on Windows. Compiled on macOS only; Windows keeps the in-window
//! bar (docs/07-UI-SPEC.md).

#![cfg(target_os = "macos")]

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    NewProject,
    OpenProject,
    ImportFootage,
    Save,
    ExportComp,
    ExportYouTube1080,
    ExportVertical,
    ShareExport50,
    ShareExport10,
    Undo,
    Redo,
    NewComposition,
    AddSolidLayer,
    AddTextLayer,
    AddCameraLayer,
    AddAdjustmentLayer,
    AddSequenceLayer,
    CutClip,
    DeleteClip,
    DetectBeats,
    DetectBeatsMore,
    AddMarker,
    ClearBeatMarkers,
    AddMaskRectangle,
    AddMaskEllipse,
    AddMaskStar,
    CompSettings,
    ResetWorkspace,
}

pub struct NativeMenu {
    _menu: Menu,
    undo: MenuItem,
    redo: MenuItem,
}

fn item(id: &str, label: &str, accel: Option<Accelerator>) -> MenuItem {
    MenuItem::with_id(id, label, true, accel)
}

fn cmd(code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(Some(Modifiers::META), code))
}

fn cmd_shift(code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(
        Some(Modifiers::META | Modifiers::SHIFT),
        code,
    ))
}

impl NativeMenu {
    /// Build and install the menu on the running NSApplication.
    /// Must be called on the main thread during start-up.
    pub fn install() -> Result<Self, muda::Error> {
        let menu = Menu::new();

        let about = AboutMetadata {
            name: Some("Luminal".into()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            comments: Some("Named for Edo luminal: glass, cut precisely.".into()),
            ..Default::default()
        };
        let app = Submenu::new("Luminal", true);
        app.append_items(&[
            &PredefinedMenuItem::about(None, Some(about)),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ])?;

        let file = Submenu::new("File", true);
        file.append_items(&[
            &item("file.new", "New project", cmd(Code::KeyN)),
            &item("file.open", "Open project…", cmd(Code::KeyO)),
            &item("file.import", "Import footage…", cmd(Code::KeyI)),
            &PredefinedMenuItem::separator(),
            &item("file.save", "Save", cmd(Code::KeyS)),
            &item("file.export", "Export comp…", cmd_shift(Code::KeyE)),
            &item("file.export.yt1080", "Export for YouTube (1080p)…", None),
            &item("file.export.vertical", "Export vertical (1080×1920)…", None),
            &item("file.share50", "Export for sharing (50 MB)…", None),
            &item("file.share10", "Export for sharing (10 MB)…", None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(None),
        ])?;

        let undo = item("edit.undo", "Undo", cmd(Code::KeyZ));
        let redo = item("edit.redo", "Redo", cmd_shift(Code::KeyZ));
        let edit = Submenu::new("Edit", true);
        edit.append_items(&[
            &undo,
            &redo,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ])?;

        let comp = Submenu::new("Composition", true);
        comp.append_items(&[
            &item("comp.new", "New composition", cmd_shift(Code::KeyN)),
            &item("comp.solid", "Add solid layer", None),
            &item("comp.text", "Add text layer", None),
            &item("comp.camera", "Add camera layer", None),
            &item("comp.adjustment", "Add adjustment layer", None),
            &item("comp.sequence", "Add sequence layer", None),
            &item("comp.cut", "Cut clip at playhead", cmd_shift(Code::KeyD)),
            &item("comp.delclip", "Delete clip at playhead", None),
            &item("comp.beats", "Detect beats", None),
            &item("comp.beats.more", "Detect beats (more markers)", None),
            &item("comp.marker", "Add marker at playhead", None),
            &item("comp.clearbeats", "Clear beat markers", None),
            &item("comp.settings", "Composition settings…", None),
        ])?;
        let mask = Submenu::new("Add mask", true);
        mask.append_items(&[
            &item("comp.mask.rect", "Rectangle", None),
            &item("comp.mask.ellipse", "Ellipse", None),
            &item("comp.mask.star", "Star", None),
        ])?;
        comp.append(&mask)?;

        let window = Submenu::new("Window", true);
        window.append_items(&[
            &item("window.reset", "Reset workspace", None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::fullscreen(None),
        ])?;

        menu.append_items(&[&app, &file, &edit, &comp, &window])?;
        menu.init_for_nsapp();

        Ok(Self {
            _menu: menu,
            undo,
            redo,
        })
    }

    /// Drain pending menu events into actions. Called once per UI frame.
    pub fn poll(&self) -> Vec<MenuAction> {
        let mut actions = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let action = match event.id().0.as_str() {
                "file.new" => Some(MenuAction::NewProject),
                "file.open" => Some(MenuAction::OpenProject),
                "file.import" => Some(MenuAction::ImportFootage),
                "file.save" => Some(MenuAction::Save),
                "file.export" => Some(MenuAction::ExportComp),
                "file.export.yt1080" => Some(MenuAction::ExportYouTube1080),
                "file.export.vertical" => Some(MenuAction::ExportVertical),
                "file.share50" => Some(MenuAction::ShareExport50),
                "file.share10" => Some(MenuAction::ShareExport10),
                "edit.undo" => Some(MenuAction::Undo),
                "edit.redo" => Some(MenuAction::Redo),
                "comp.new" => Some(MenuAction::NewComposition),
                "comp.solid" => Some(MenuAction::AddSolidLayer),
                "comp.text" => Some(MenuAction::AddTextLayer),
                "comp.camera" => Some(MenuAction::AddCameraLayer),
                "comp.adjustment" => Some(MenuAction::AddAdjustmentLayer),
                "comp.sequence" => Some(MenuAction::AddSequenceLayer),
                "comp.cut" => Some(MenuAction::CutClip),
                "comp.delclip" => Some(MenuAction::DeleteClip),
                "comp.beats" => Some(MenuAction::DetectBeats),
                "comp.beats.more" => Some(MenuAction::DetectBeatsMore),
                "comp.marker" => Some(MenuAction::AddMarker),
                "comp.clearbeats" => Some(MenuAction::ClearBeatMarkers),
                "comp.mask.rect" => Some(MenuAction::AddMaskRectangle),
                "comp.mask.ellipse" => Some(MenuAction::AddMaskEllipse),
                "comp.mask.star" => Some(MenuAction::AddMaskStar),
                "comp.settings" => Some(MenuAction::CompSettings),
                "window.reset" => Some(MenuAction::ResetWorkspace),
                _ => None,
            };
            actions.extend(action);
        }
        actions
    }

    /// Keep native enabled-states in step with the document store.
    pub fn sync(&self, can_undo: bool, can_redo: bool) {
        self.undo.set_enabled(can_undo);
        self.redo.set_enabled(can_redo);
    }
}
