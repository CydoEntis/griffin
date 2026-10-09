//! The one place that looks at `KeyCode`: turns key events into an `Action` or typed
//! text, using the spec's default bindings overridden by `[keys]` in `config.toml`.

use std::collections::HashMap;
use std::fmt;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::buffer::movement::Motion;
use crate::config::KeysConfig;

/// Something a key can trigger. Each feature adds its variant here, a name in
/// `Action::name` and a line in `DEFAULT_BINDINGS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Quit,
    Save,
    /// Esc: closes whatever prompt or popup is open.
    Cancel,
    Move(Motion),
    Newline,
    Backspace,
    Delete,
    Tab,
    Undo,
    Redo,
    /// Shift+movement: moves the cursor and extends the selection.
    Select(Motion),
    SelectAll,
    Copy,
    Cut,
    Paste,
    /// Shows or hides the file tree.
    ToggleTree,
    /// Moves focus between the file tree and the editor.
    FocusTree,
    /// Tree focus only: prompts for a file name and creates it.
    TreeNewFile,
    /// Tree focus only: prompts for a folder name and creates it.
    TreeNewFolder,
    /// Tree focus only: prompts for a new name for the selected entry.
    TreeRename,
    /// Tree focus only: asks, then moves the selected entry to the OS trash.
    TreeDelete,
    PrevTab,
    NextTab,
    /// Jumps to tab n, counting from 1 as the keys do.
    GoToTab(u8),
    /// Closes the active tab, asking first when it has unsaved changes.
    CloseTab,
    /// Opens a new untitled tab.
    NewFile,
    /// Prompts for a path and saves the active buffer there.
    SaveAs,
    /// Opens a second editor split on the right, or closes it.
    ToggleSplit,
    /// Moves focus to the next of tree, left split and right split.
    CycleFocus,
    /// Opens the fuzzy picker over the project's files.
    GoToFile,
    /// Opens the cast palette with `:` typed, to go to a line.
    GoToLine,
    /// Opens the find bar over the active buffer.
    Find,
    /// Find bar only: moves to the next match, wrapping.
    FindNext,
    /// Find bar only: moves to the previous match, wrapping.
    FindPrev,
    /// Find bar only: toggles case-sensitive matching.
    FindCase,
    /// Find bar only: toggles regex matching.
    FindRegex,
    /// Opens the find bar with its Replace field.
    Replace,
    /// Find bar only: replaces every match and reports how many.
    ReplaceAll,
    /// Opens the project search panel.
    ProjectSearch,
    /// Project search only: asks, then replaces the matches in every listed file.
    ProjectReplace,
    /// Runs a `[[run]]` command from `.glyph.toml`, asking which when several.
    Run,
    /// Shows or hides the run panel.
    ToggleRunPanel,
    /// Stops the running command and everything it started.
    StopRun,
    /// Stops the command, if it's running, and starts it again.
    RestartRun,
    /// Moves the cursor to the next diagnostic in the buffer, wrapping around.
    NextDiagnostic,
    /// Moves the cursor to the previous diagnostic in the buffer, wrapping around.
    PrevDiagnostic,
    /// Sets a breakpoint on the cursor's line, or takes away the one there.
    ToggleBreakpoint,
    /// Takes away every breakpoint, in open and closed files alike.
    ClearBreakpoints,
    /// Builds, then starts the active file's program under its debugger.
    DebugStart,
    /// Ends the debug session and the program with it.
    DebugStop,
    /// While paused: runs the stopped thread to its next line in this function.
    StepOver,
    /// While paused: runs the stopped thread into the call on its line.
    StepInto,
    /// While paused: runs the stopped thread until its function returns.
    StepOut,
    /// Asks the language server where the symbol under the cursor is defined and
    /// goes there.
    GoToDefinition,
    /// Returns to where the cursor was before the last go to definition.
    JumpBack,
    /// Asks the language server about the symbol under the cursor and shows the
    /// answer in a popup.
    Hover,
    /// Asks the language server for completions at the cursor and shows them in
    /// a popup.
    Complete,
    /// Opens the folder browser, to open another folder as the project.
    OpenDirectory,
    /// Folder browser only: opens the folder it shows, whatever is selected.
    OpenFolderHere,
    /// Opens the language server catalog: each server and whether it's installed.
    LanguageServers,
    /// Catalog only: copies the selected server's install command.
    CatalogCopy,
    /// Opens `config.toml` in a tab, creating it from a template if missing.
    Settings,
    /// Splash only: moves the selection up a row, wrapping.
    SplashUp,
    /// Splash only: moves the selection down a row, wrapping.
    SplashDown,
    /// Splash only: runs the selected row.
    SplashRun,
    /// Splash only: runs *New file*.
    SplashNewFile,
    /// Splash only: runs *New directory*.
    SplashNewDirectory,
    /// Splash only: runs *Open directory*.
    SplashOpenDirectory,
    /// Splash only: leaves the splash for the empty untitled buffer.
    SplashDismiss,
    /// Splash only: quits as Ctrl+Q does.
    SplashQuit,
    /// Debug panel only: moves the selection up a row.
    DebugUp,
    /// Debug panel only: moves the selection down a row.
    DebugDown,
    /// Debug panel only: on a frame, shows its variables and opens its file at
    /// its line; on a variable, expands or collapses it.
    DebugActivate,
    /// Debug panel only: expands the selected variable.
    DebugExpand,
    /// Debug panel only: collapses the selected variable, or goes to its parent.
    DebugCollapse,
    /// Debug panel only: moves between the call stack and the variables.
    DebugSwitchPane,
}

/// Where a binding applies. Tree bindings are plain letters, so they only count
/// while the tree has focus; everywhere else those letters are typed text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Tree,
    /// While the find bar is open: its keys win over the global ones.
    Find,
    /// While project search is open: like `Find`, with its own keys checked
    /// first, so Alt+A can replace across the project here and in the find bar
    /// replace only in the file.
    Search,
    /// While the folder browser is open: its keys win over the global ones.
    Folders,
    /// While the language server catalog is open: its letters are its own, as
    /// nothing is typed into it.
    Catalog,
    /// While the splash has focus: its letters and arrows are its own.
    Splash,
    /// While the debug panel has focus: its arrows, Enter and Tab are its own.
    Debug,
}

impl Action {
    const ALL: &[Action] = &[
        Action::Quit,
        Action::Save,
        Action::Cancel,
        Action::Move(Motion::Left),
        Action::Move(Motion::Right),
        Action::Move(Motion::Up),
        Action::Move(Motion::Down),
        Action::Move(Motion::LineStart),
        Action::Move(Motion::LineEnd),
        Action::Move(Motion::PageUp),
        Action::Move(Motion::PageDown),
        Action::Move(Motion::WordLeft),
        Action::Move(Motion::WordRight),
        Action::Move(Motion::DocStart),
        Action::Move(Motion::DocEnd),
        Action::Newline,
        Action::Backspace,
        Action::Delete,
        Action::Tab,
        Action::Undo,
        Action::Redo,
        Action::Select(Motion::Left),
        Action::Select(Motion::Right),
        Action::Select(Motion::Up),
        Action::Select(Motion::Down),
        Action::Select(Motion::LineStart),
        Action::Select(Motion::LineEnd),
        Action::Select(Motion::PageUp),
        Action::Select(Motion::PageDown),
        Action::Select(Motion::WordLeft),
        Action::Select(Motion::WordRight),
        Action::Select(Motion::DocStart),
        Action::Select(Motion::DocEnd),
        Action::SelectAll,
        Action::Copy,
        Action::Cut,
        Action::Paste,
        Action::ToggleTree,
        Action::FocusTree,
        Action::TreeNewFile,
        Action::TreeNewFolder,
        Action::TreeRename,
        Action::TreeDelete,
        Action::PrevTab,
        Action::NextTab,
        Action::GoToTab(1),
        Action::GoToTab(2),
        Action::GoToTab(3),
        Action::GoToTab(4),
        Action::GoToTab(5),
        Action::GoToTab(6),
        Action::GoToTab(7),
        Action::GoToTab(8),
        Action::GoToTab(9),
        Action::CloseTab,
        Action::NewFile,
        Action::SaveAs,
        Action::ToggleSplit,
        Action::CycleFocus,
        Action::GoToFile,
        Action::GoToLine,
        Action::Find,
        Action::FindNext,
        Action::FindPrev,
        Action::FindCase,
        Action::FindRegex,
        Action::Replace,
        Action::ReplaceAll,
        Action::ProjectSearch,
        Action::ProjectReplace,
        Action::Run,
        Action::ToggleRunPanel,
        Action::StopRun,
        Action::RestartRun,
        Action::NextDiagnostic,
        Action::PrevDiagnostic,
        Action::ToggleBreakpoint,
        Action::ClearBreakpoints,
        Action::DebugStart,
        Action::DebugStop,
        Action::StepOver,
        Action::StepInto,
        Action::StepOut,
        Action::GoToDefinition,
        Action::JumpBack,
        Action::Hover,
        Action::Complete,
        Action::OpenDirectory,
        Action::OpenFolderHere,
        Action::LanguageServers,
        Action::CatalogCopy,
        Action::Settings,
        Action::SplashUp,
        Action::SplashDown,
        Action::SplashRun,
        Action::SplashNewFile,
        Action::SplashNewDirectory,
        Action::SplashOpenDirectory,
        Action::SplashDismiss,
        Action::SplashQuit,
        Action::DebugUp,
        Action::DebugDown,
        Action::DebugActivate,
        Action::DebugExpand,
        Action::DebugCollapse,
        Action::DebugSwitchPane,
    ];

    pub fn scope(self) -> Scope {
        match self {
            Action::TreeNewFile
            | Action::TreeNewFolder
            | Action::TreeRename
            | Action::TreeDelete => Scope::Tree,
            Action::FindNext
            | Action::FindPrev
            | Action::FindCase
            | Action::FindRegex
            | Action::ReplaceAll => Scope::Find,
            Action::ProjectReplace => Scope::Search,
            Action::OpenFolderHere => Scope::Folders,
            Action::CatalogCopy => Scope::Catalog,
            Action::SplashUp
            | Action::SplashDown
            | Action::SplashRun
            | Action::SplashNewFile
            | Action::SplashNewDirectory
            | Action::SplashOpenDirectory
            | Action::SplashDismiss
            | Action::SplashQuit => Scope::Splash,
            Action::DebugUp
            | Action::DebugDown
            | Action::DebugActivate
            | Action::DebugExpand
            | Action::DebugCollapse
            | Action::DebugSwitchPane => Scope::Debug,
            _ => Scope::Global,
        }
    }

    /// The name used on the left of `[keys]`.
    pub fn name(self) -> &'static str {
        match self {
            Action::Quit => "quit",
            Action::Save => "save",
            Action::Cancel => "cancel",
            Action::Move(motion) => match motion {
                Motion::Left => "move_left",
                Motion::Right => "move_right",
                Motion::Up => "move_up",
                Motion::Down => "move_down",
                Motion::LineStart => "line_start",
                Motion::LineEnd => "line_end",
                Motion::PageUp => "page_up",
                Motion::PageDown => "page_down",
                Motion::WordLeft => "word_left",
                Motion::WordRight => "word_right",
                Motion::DocStart => "doc_start",
                Motion::DocEnd => "doc_end",
            },
            Action::Newline => "newline",
            Action::Backspace => "backspace",
            Action::Delete => "delete",
            Action::Tab => "tab",
            Action::Undo => "undo",
            Action::Redo => "redo",
            Action::Select(motion) => match motion {
                Motion::Left => "select_left",
                Motion::Right => "select_right",
                Motion::Up => "select_up",
                Motion::Down => "select_down",
                Motion::LineStart => "select_line_start",
                Motion::LineEnd => "select_line_end",
                Motion::PageUp => "select_page_up",
                Motion::PageDown => "select_page_down",
                Motion::WordLeft => "select_word_left",
                Motion::WordRight => "select_word_right",
                Motion::DocStart => "select_doc_start",
                Motion::DocEnd => "select_doc_end",
            },
            Action::SelectAll => "select_all",
            Action::Copy => "copy",
            Action::Cut => "cut",
            Action::Paste => "paste",
            Action::ToggleTree => "toggle_tree",
            Action::FocusTree => "focus_tree",
            Action::TreeNewFile => "tree_new_file",
            Action::TreeNewFolder => "tree_new_folder",
            Action::TreeRename => "tree_rename",
            Action::TreeDelete => "tree_delete",
            Action::PrevTab => "prev_tab",
            Action::NextTab => "next_tab",
            Action::GoToTab(n) => match n {
                1 => "tab_1",
                2 => "tab_2",
                3 => "tab_3",
                4 => "tab_4",
                5 => "tab_5",
                6 => "tab_6",
                7 => "tab_7",
                8 => "tab_8",
                // `ALL` only holds 1..=9, so this arm is tab 9.
                _ => "tab_9",
            },
            Action::CloseTab => "close_tab",
            Action::NewFile => "new_file",
            Action::SaveAs => "save_as",
            Action::ToggleSplit => "toggle_split",
            Action::CycleFocus => "cycle_focus",
            Action::GoToFile => "go_to_file",
            Action::GoToLine => "go_to_line",
            Action::Find => "find",
            Action::FindNext => "find_next",
            Action::FindPrev => "find_prev",
            Action::FindCase => "find_case",
            Action::FindRegex => "find_regex",
            Action::Replace => "replace",
            Action::ReplaceAll => "replace_all",
            Action::ProjectSearch => "project_search",
            Action::ProjectReplace => "project_replace",
            Action::Run => "run",
            Action::ToggleRunPanel => "toggle_run_panel",
            Action::StopRun => "stop_run",
            Action::RestartRun => "restart_run",
            Action::NextDiagnostic => "next_diagnostic",
            Action::PrevDiagnostic => "prev_diagnostic",
            Action::ToggleBreakpoint => "toggle_breakpoint",
            Action::ClearBreakpoints => "clear_breakpoints",
            Action::DebugStart => "debug_start",
            Action::DebugStop => "debug_stop",
            Action::StepOver => "step_over",
            Action::StepInto => "step_into",
            Action::StepOut => "step_out",
            Action::GoToDefinition => "go_to_definition",
            Action::JumpBack => "jump_back",
            Action::Hover => "hover",
            Action::Complete => "complete",
            Action::OpenDirectory => "open_directory",
            Action::OpenFolderHere => "open_folder_here",
            Action::LanguageServers => "language_servers",
            Action::Settings => "settings",
            Action::CatalogCopy => "catalog_copy",
            Action::SplashUp => "splash_up",
            Action::SplashDown => "splash_down",
            Action::SplashRun => "splash_run",
            Action::SplashNewFile => "splash_new_file",
            Action::SplashNewDirectory => "splash_new_directory",
            Action::SplashOpenDirectory => "splash_open_directory",
            Action::SplashDismiss => "splash_dismiss",
            Action::SplashQuit => "splash_quit",
            Action::DebugUp => "debug_up",
            Action::DebugDown => "debug_down",
            Action::DebugActivate => "debug_activate",
            Action::DebugExpand => "debug_expand",
            Action::DebugCollapse => "debug_collapse",
            Action::DebugSwitchPane => "debug_switch_pane",
        }
    }

    /// What the cast palette calls the action: its name in plain words.
    pub fn title(self) -> &'static str {
        match self {
            Action::Quit => "Quit",
            Action::Save => "Save",
            Action::Cancel => "Cancel",
            Action::Move(motion) => match motion {
                Motion::Left => "Move left",
                Motion::Right => "Move right",
                Motion::Up => "Move up",
                Motion::Down => "Move down",
                Motion::LineStart => "Go to line start",
                Motion::LineEnd => "Go to line end",
                Motion::PageUp => "Page up",
                Motion::PageDown => "Page down",
                Motion::WordLeft => "Word left",
                Motion::WordRight => "Word right",
                Motion::DocStart => "Go to start of file",
                Motion::DocEnd => "Go to end of file",
            },
            Action::Newline => "Insert line break",
            Action::Backspace => "Delete back",
            Action::Delete => "Delete forward",
            Action::Tab => "Indent",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::Select(motion) => match motion {
                Motion::Left => "Select left",
                Motion::Right => "Select right",
                Motion::Up => "Select up",
                Motion::Down => "Select down",
                Motion::LineStart => "Select to line start",
                Motion::LineEnd => "Select to line end",
                Motion::PageUp => "Select page up",
                Motion::PageDown => "Select page down",
                Motion::WordLeft => "Select word left",
                Motion::WordRight => "Select word right",
                Motion::DocStart => "Select to start of file",
                Motion::DocEnd => "Select to end of file",
            },
            Action::SelectAll => "Select all",
            Action::Copy => "Copy",
            Action::Cut => "Cut",
            Action::Paste => "Paste",
            Action::ToggleTree => "Toggle file tree",
            Action::FocusTree => "Focus file tree",
            Action::TreeNewFile => "New file in tree",
            Action::TreeNewFolder => "New folder in tree",
            Action::TreeRename => "Rename in tree",
            Action::TreeDelete => "Delete in tree",
            Action::PrevTab => "Previous tab",
            Action::NextTab => "Next tab",
            Action::GoToTab(n) => match n {
                1 => "Go to tab 1",
                2 => "Go to tab 2",
                3 => "Go to tab 3",
                4 => "Go to tab 4",
                5 => "Go to tab 5",
                6 => "Go to tab 6",
                7 => "Go to tab 7",
                8 => "Go to tab 8",
                // `ALL` only holds 1..=9, so this arm is tab 9.
                _ => "Go to tab 9",
            },
            Action::CloseTab => "Close tab",
            Action::NewFile => "New file",
            Action::SaveAs => "Save as…",
            Action::ToggleSplit => "Split right",
            Action::CycleFocus => "Cycle focus",
            Action::GoToFile => "Go to file…",
            Action::GoToLine => "Go to line…",
            Action::Find => "Find…",
            Action::FindNext => "Find next",
            Action::FindPrev => "Find previous",
            Action::FindCase => "Toggle match case",
            Action::FindRegex => "Toggle regex",
            Action::Replace => "Replace…",
            Action::ReplaceAll => "Replace all",
            Action::ProjectSearch => "Search project…",
            Action::ProjectReplace => "Replace in project",
            Action::Run => "Start run…",
            Action::ToggleRunPanel => "Toggle run panel",
            Action::StopRun => "Stop run",
            Action::RestartRun => "Restart run",
            Action::NextDiagnostic => "Next diagnostic",
            Action::PrevDiagnostic => "Previous diagnostic",
            Action::ToggleBreakpoint => "Toggle breakpoint",
            Action::ClearBreakpoints => "Clear breakpoints",
            Action::DebugStart => "Start debugging",
            Action::DebugStop => "Stop debugging",
            Action::StepOver => "Step over",
            Action::StepInto => "Step into",
            Action::StepOut => "Step out",
            Action::GoToDefinition => "Go to definition",
            Action::JumpBack => "Jump back",
            Action::Hover => "Show hover",
            Action::Complete => "Complete",
            Action::OpenDirectory => "Open directory",
            Action::OpenFolderHere => "Open this folder",
            Action::LanguageServers => "Language servers",
            Action::Settings => "Settings",
            Action::CatalogCopy => "Catalog: copy command",
            Action::SplashUp => "Splash: up",
            Action::SplashDown => "Splash: down",
            Action::SplashRun => "Splash: run selected",
            Action::SplashNewFile => "Splash: new file",
            Action::SplashNewDirectory => "Splash: new directory",
            Action::SplashOpenDirectory => "Splash: open directory",
            Action::SplashDismiss => "Splash: close",
            Action::SplashQuit => "Splash: quit",
            Action::DebugUp => "Debug panel: up",
            Action::DebugDown => "Debug panel: down",
            Action::DebugActivate => "Debug panel: open frame or expand",
            Action::DebugExpand => "Debug panel: expand",
            Action::DebugCollapse => "Debug panel: collapse",
            Action::DebugSwitchPane => "Debug panel: stack or variables",
        }
    }

    /// The actions the cast palette lists: every global one, in `ALL` order,
    /// except Esc's, which in the palette only closes it.
    pub fn commands() -> impl Iterator<Item = Action> {
        Action::ALL
            .iter()
            .copied()
            .filter(|a| a.scope() == Scope::Global && *a != Action::Cancel)
    }

    fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }
}

/// The spec's "Default keymap" table plus R4's movement, R6's editing, R10's
/// selection, R14's tree keys, R16's tab keys, R17's split keys, R18's picker
/// keys, R20's find bar keys, R21's Replace All, R22's project search, R23's
/// project replace, R26's run keys, R27's stop and restart, R30's diagnostic
/// jumps, R31's definition keys, R32's hover and R33's completion, one line per
/// binding.
const DEFAULT_BINDINGS: &[(Action, &str)] = &[
    (Action::Quit, "ctrl+q"),
    (Action::Save, "ctrl+s"),
    (Action::Cancel, "esc"),
    (Action::Move(Motion::Left), "left"),
    (Action::Move(Motion::Right), "right"),
    (Action::Move(Motion::Up), "up"),
    (Action::Move(Motion::Down), "down"),
    (Action::Move(Motion::LineStart), "home"),
    (Action::Move(Motion::LineEnd), "end"),
    (Action::Move(Motion::PageUp), "pageup"),
    (Action::Move(Motion::PageDown), "pagedown"),
    (Action::Move(Motion::WordLeft), "ctrl+left"),
    (Action::Move(Motion::WordRight), "ctrl+right"),
    (Action::Move(Motion::DocStart), "ctrl+home"),
    (Action::Move(Motion::DocEnd), "ctrl+end"),
    (Action::Newline, "enter"),
    (Action::Backspace, "backspace"),
    (Action::Delete, "delete"),
    (Action::Tab, "tab"),
    (Action::Undo, "ctrl+z"),
    (Action::Redo, "ctrl+y"),
    (Action::Select(Motion::Left), "shift+left"),
    (Action::Select(Motion::Right), "shift+right"),
    (Action::Select(Motion::Up), "shift+up"),
    (Action::Select(Motion::Down), "shift+down"),
    (Action::Select(Motion::LineStart), "shift+home"),
    (Action::Select(Motion::LineEnd), "shift+end"),
    (Action::Select(Motion::PageUp), "shift+pageup"),
    (Action::Select(Motion::PageDown), "shift+pagedown"),
    (Action::Select(Motion::WordLeft), "ctrl+shift+left"),
    (Action::Select(Motion::WordRight), "ctrl+shift+right"),
    (Action::Select(Motion::DocStart), "ctrl+shift+home"),
    (Action::Select(Motion::DocEnd), "ctrl+shift+end"),
    (Action::SelectAll, "ctrl+a"),
    (Action::Copy, "ctrl+c"),
    (Action::Cut, "ctrl+x"),
    (Action::Paste, "ctrl+v"),
    (Action::ToggleTree, "ctrl+b"),
    (Action::FocusTree, "ctrl+e"),
    (Action::TreeNewFile, "a"),
    (Action::TreeNewFolder, "shift+a"),
    (Action::TreeRename, "r"),
    (Action::TreeDelete, "d"),
    (Action::PrevTab, "alt+,"),
    (Action::NextTab, "alt+."),
    (Action::GoToTab(1), "alt+1"),
    (Action::GoToTab(2), "alt+2"),
    (Action::GoToTab(3), "alt+3"),
    (Action::GoToTab(4), "alt+4"),
    (Action::GoToTab(5), "alt+5"),
    (Action::GoToTab(6), "alt+6"),
    (Action::GoToTab(7), "alt+7"),
    (Action::GoToTab(8), "alt+8"),
    (Action::GoToTab(9), "alt+9"),
    (Action::CloseTab, "ctrl+w"),
    (Action::NewFile, "ctrl+n"),
    (Action::SaveAs, "alt+s"),
    (Action::ToggleSplit, "alt+v"),
    (Action::CycleFocus, "f6"),
    (Action::GoToFile, "ctrl+p"),
    (Action::GoToLine, "ctrl+g"),
    (Action::Find, "ctrl+f"),
    (Action::FindNext, "enter"),
    (Action::FindPrev, "shift+enter"),
    (Action::FindCase, "alt+c"),
    (Action::FindRegex, "alt+r"),
    (Action::Replace, "ctrl+r"),
    (Action::ReplaceAll, "alt+a"),
    (Action::ProjectSearch, "alt+f"),
    // Not Alt+Enter: Windows Terminal keeps that for full screen.
    (Action::ProjectReplace, "alt+a"),
    (Action::Run, "f5"),
    (Action::ToggleRunPanel, "f4"),
    (Action::StopRun, "shift+f5"),
    (Action::RestartRun, "ctrl+f5"),
    (Action::NextDiagnostic, "f8"),
    (Action::PrevDiagnostic, "shift+f8"),
    (Action::ToggleBreakpoint, "f9"),
    // F5 and its Shift and Ctrl forms are the run panel's (glyph-debugger spec).
    (Action::DebugStart, "alt+f5"),
    // F6 is CycleFocus.
    (Action::DebugStop, "alt+f6"),
    // F11 is Windows Terminal's full screen key and never reaches Glyph, so
    // stepping lives on F10 and its Alt and Shift forms (glyph-debugger spec).
    (Action::StepOver, "f10"),
    (Action::StepInto, "alt+f10"),
    (Action::StepOut, "shift+f10"),
    (Action::GoToDefinition, "f12"),
    (Action::JumpBack, "alt+left"),
    (Action::Hover, "alt+k"),
    (Action::Complete, "alt+/"),
    // Many Unix terminals send Ctrl+Enter as plain Enter, so the browser's first
    // row does the same and this is only a shortcut for it.
    (Action::OpenFolderHere, "ctrl+enter"),
    // Nothing is typed into the catalog, so a plain letter is free there
    // (glyph-catalog spec C2).
    (Action::CatalogCopy, "c"),
    // The splash's own keys (glyph-splash spec S3): letters are free there, as
    // nothing is typed into the splash.
    (Action::SplashUp, "up"),
    (Action::SplashDown, "down"),
    (Action::SplashRun, "enter"),
    (Action::SplashNewFile, "n"),
    (Action::SplashNewDirectory, "d"),
    (Action::SplashOpenDirectory, "o"),
    (Action::SplashDismiss, "esc"),
    (Action::SplashQuit, "q"),
    // The debug panel's own keys (glyph-debugger spec D7): nothing is typed
    // into it, so the arrows, Enter and Tab are free there.
    (Action::DebugUp, "up"),
    (Action::DebugDown, "down"),
    (Action::DebugActivate, "enter"),
    (Action::DebugExpand, "right"),
    (Action::DebugCollapse, "left"),
    (Action::DebugSwitchPane, "tab"),
];

/// Actions with no default key, reached from the cast palette or bound in
/// `[keys]`: the spec gives them none. Only the check that every action is
/// accounted for reads it.
#[cfg(test)]
const UNBOUND: &[Action] = &[
    Action::OpenDirectory,
    Action::ClearBreakpoints,
    Action::LanguageServers,
    Action::Settings,
];

/// What a key event means to the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Action(Action),
    /// A printable character typed without Ctrl or Alt.
    Text(char),
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapError(String);

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeymapError {}

/// A key with its modifiers, normalised so that what the terminal reports and what
/// the config names compare equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    code: KeyCode,
    mods: KeyModifiers,
}

#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: HashMap<Key, Action>,
    /// Checked before `bindings` while the tree has focus.
    tree: HashMap<Key, Action>,
    /// Checked before `bindings` while the find bar or project search is open.
    find: HashMap<Key, Action>,
    /// Checked before `find` while project search is open.
    search: HashMap<Key, Action>,
    /// Checked before `bindings` while the folder browser is open.
    folders: HashMap<Key, Action>,
    /// Checked before `bindings` while the language server catalog is open.
    catalog: HashMap<Key, Action>,
    /// Checked before `bindings` while the splash has focus.
    splash: HashMap<Key, Action>,
    /// Checked before `bindings` while the debug panel has focus.
    debug: HashMap<Key, Action>,
}

impl Default for Keymap {
    fn default() -> Self {
        // Every entry in DEFAULT_BINDINGS is a valid key, which the
        // `defaults_build` test checks, so an empty `[keys]` can't fail.
        Keymap::new(&KeysConfig::new()).expect("default bindings parse")
    }
}

impl Keymap {
    /// Defaults, with every action named in `keys` taking exactly the keys given
    /// there instead of its defaults.
    pub fn new(keys: &KeysConfig) -> Result<Self, KeymapError> {
        let mut overrides = HashMap::new();
        for (name, binding) in keys {
            let action = Action::from_name(name)
                .ok_or_else(|| KeymapError(format!("[keys]: unknown action {name:?}")))?;
            let parsed = binding
                .keys()
                .iter()
                .map(|k| parse_key(k).map_err(|e| KeymapError(format!("[keys] {name}: {e}"))))
                .collect::<Result<Vec<_>, _>>()?;
            overrides.insert(action, parsed);
        }

        let mut map = Keymap {
            bindings: HashMap::new(),
            tree: HashMap::new(),
            find: HashMap::new(),
            search: HashMap::new(),
            folders: HashMap::new(),
            catalog: HashMap::new(),
            splash: HashMap::new(),
            debug: HashMap::new(),
        };
        for &(action, notation) in DEFAULT_BINDINGS {
            if overrides.contains_key(&action) {
                continue;
            }
            let key = parse_key(notation)
                .map_err(|e| KeymapError(format!("default binding for {}: {e}", action.name())))?;
            map.table(action.scope()).insert(key, action);
        }
        // Inserted last so a user's key wins over another action's default.
        for (action, keys) in overrides {
            for key in keys {
                map.table(action.scope()).insert(key, action);
            }
        }
        Ok(map)
    }

    fn table(&mut self, scope: Scope) -> &mut HashMap<Key, Action> {
        match scope {
            Scope::Global => &mut self.bindings,
            Scope::Tree => &mut self.tree,
            Scope::Find => &mut self.find,
            Scope::Search => &mut self.search,
            Scope::Folders => &mut self.folders,
            Scope::Catalog => &mut self.catalog,
            Scope::Splash => &mut self.splash,
            Scope::Debug => &mut self.debug,
        }
    }

    /// The key that runs `action` anywhere, as the cast palette shows it
    /// (`Alt+V`), or `None` when it has no global key. With several keys the
    /// shortest label wins, then the first in order, so the pick is stable.
    pub fn key_label(&self, action: Action) -> Option<String> {
        self.bindings
            .iter()
            .filter(|&(_, &bound)| bound == action)
            .map(|(&key, _)| key_label(key))
            .min_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)))
    }

    /// What `event` means outside the tree.
    #[cfg(test)]
    pub fn resolve(&self, event: &KeyEvent) -> Input {
        self.resolve_in(event, Scope::Global)
    }

    /// What `event` means in `scope`: tree bindings win while the tree has focus,
    /// find bar bindings while the find bar is open, project search's own
    /// bindings, then the find bar's, while project search is open, and the
    /// splash's while it has focus.
    pub fn resolve_in(&self, event: &KeyEvent, scope: Scope) -> Input {
        if event.kind == KeyEventKind::Release {
            return Input::Ignored;
        }
        let key = normalize(event.code, event.modifiers);
        let scoped = match scope {
            Scope::Global => None,
            Scope::Tree => self.tree.get(&key),
            Scope::Find => self.find.get(&key),
            Scope::Search => self.search.get(&key).or_else(|| self.find.get(&key)),
            Scope::Folders => self.folders.get(&key),
            Scope::Catalog => self.catalog.get(&key),
            Scope::Splash => self.splash.get(&key),
            Scope::Debug => self.debug.get(&key),
        };
        if let Some(&action) = scoped {
            return Input::Action(action);
        }
        if let Some(&action) = self.bindings.get(&key) {
            return Input::Action(action);
        }
        match event.code {
            KeyCode::Char(c)
                if !c.is_control()
                    && !event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                Input::Text(c)
            }
            _ => Input::Ignored,
        }
    }
}

/// The text of a burst of key events that arrived together, when they look like a
/// paste rather than typing: crossterm can't report bracketed paste on Windows, so
/// a paste there (Windows Terminal's own Ctrl+V included) arrives as a run of key
/// presses instead. A burst counts as pasted when every press is plain text, Enter
/// or Tab and it holds a line break or tab; nobody types a newline in the same
/// instant as the text before it, while a burst of plain letters is left as typing
/// so fast typists keep their undo steps.
#[cfg_attr(
    not(any(windows, test)),
    expect(dead_code, reason = "only Windows needs to recover pastes from keys")
)]
pub fn burst_as_paste(events: &[Event]) -> Option<String> {
    let mut text = String::new();
    let mut presses = 0;
    for event in events {
        let Event::Key(key) = event else {
            return None;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        match key.code {
            KeyCode::Char(c) if !c.is_control() => text.push(c),
            KeyCode::Enter => text.push('\n'),
            KeyCode::Tab => text.push('\t'),
            _ => return None,
        }
        presses += 1;
    }
    (presses >= 2 && text.contains(['\n', '\t'])).then_some(text)
}

/// Terminals report Shift+letter as the uppercase letter (sometimes with SHIFT,
/// sometimes without) and Shift+Tab as BackTab; fold those into one form. For other
/// characters the shifted character itself is the key, so SHIFT is dropped.
fn normalize(code: KeyCode, modifiers: KeyModifiers) -> Key {
    let mut mods = modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
    let code = match code {
        KeyCode::BackTab => {
            mods |= KeyModifiers::SHIFT;
            KeyCode::Tab
        }
        KeyCode::Char(c) if c.is_ascii_uppercase() => {
            mods |= KeyModifiers::SHIFT;
            KeyCode::Char(c.to_ascii_lowercase())
        }
        KeyCode::Char(c) if c != ' ' && !c.is_ascii_lowercase() => {
            mods -= KeyModifiers::SHIFT;
            KeyCode::Char(c)
        }
        other => other,
    };
    Key { code, mods }
}

/// Parses one key in the spec's notation: `ctrl+s`, `alt+,`, `shift+f5`, `ctrl++`.
fn parse_key(notation: &str) -> Result<Key, String> {
    let lower = notation.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Err("empty key".into());
    }
    // A trailing "+" is the plus key itself (`ctrl++`), not a separator.
    let (prefix, name) = match lower.strip_suffix("++") {
        Some(rest) => (rest, "+"),
        None if lower == "+" => ("", "+"),
        None => lower.rsplit_once('+').unwrap_or(("", lower.as_str())),
    };

    let mut mods = KeyModifiers::NONE;
    for part in prefix.split('+') {
        mods |= match part {
            "ctrl" | "control" => KeyModifiers::CONTROL,
            "alt" | "meta" => KeyModifiers::ALT,
            "shift" => KeyModifiers::SHIFT,
            "" if prefix.is_empty() => KeyModifiers::NONE,
            other => return Err(format!("unknown modifier {other:?} in {notation:?}")),
        };
    }

    let code = match name {
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "insert" | "ins" => KeyCode::Insert,
        "delete" | "del" => KeyCode::Delete,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        _ => match function_key(name) {
            Some(n) => KeyCode::F(n),
            None => {
                let mut chars = name.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if !c.is_control() => {
                        if mods.contains(KeyModifiers::SHIFT) && !c.is_ascii_alphabetic() {
                            return Err(format!(
                                "{notation:?}: write the shifted character instead of shift+{c}"
                            ));
                        }
                        KeyCode::Char(c)
                    }
                    _ => return Err(format!("unknown key {name:?} in {notation:?}")),
                }
            }
        },
    };
    Ok(normalize(code, mods))
}

/// A key as people write it: `Ctrl+Shift+Left`, `Alt+,`, `F5`.
fn key_label(key: Key) -> String {
    let mut label = String::new();
    for (modifier, name) in [
        (KeyModifiers::CONTROL, "Ctrl+"),
        (KeyModifiers::SHIFT, "Shift+"),
        (KeyModifiers::ALT, "Alt+"),
    ] {
        if key.mods.contains(modifier) {
            label.push_str(name);
        }
    }
    let name = match key.code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_uppercase().collect(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        other => format!("{other:?}"),
    };
    label.push_str(&name);
    label
}

fn function_key(name: &str) -> Option<u8> {
    let n: u8 = name.strip_prefix('f')?.parse().ok()?;
    (1..=24).contains(&n).then_some(n)
}

/// Builds the event a terminal reports for a key in `[keys]` notation, so tests
/// outside this module can send keys without naming `KeyCode`.
#[cfg(test)]
pub fn key_event(notation: &str) -> KeyEvent {
    let key = parse_key(notation).unwrap_or_else(|e| panic!("{e}"));
    let code = match key.code {
        KeyCode::Char(c) if key.mods.contains(KeyModifiers::SHIFT) => {
            KeyCode::Char(c.to_ascii_uppercase())
        }
        other => other,
    };
    KeyEvent::new(code, key.mods)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KeyBinding;

    fn keys(entries: &[(&str, KeyBinding)]) -> KeysConfig {
        entries
            .iter()
            .map(|(name, binding)| (name.to_string(), binding.clone()))
            .collect()
    }

    fn one(key: &str) -> KeyBinding {
        KeyBinding::One(key.to_string())
    }

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    /// Every key in the spec's "Default keymap" table, with the event crossterm
    /// reports for it.
    fn spec_keys() -> Vec<(&'static str, KeyEvent)> {
        let ctrl = |c| ev(KeyCode::Char(c), KeyModifiers::CONTROL);
        let alt = |c| ev(KeyCode::Char(c), KeyModifiers::ALT);
        let mut all = vec![
            ("ctrl+q", ctrl('q')),
            ("ctrl+s", ctrl('s')),
            ("alt+s", alt('s')),
            ("ctrl+n", ctrl('n')),
            ("ctrl+w", ctrl('w')),
            ("ctrl+z", ctrl('z')),
            ("ctrl+y", ctrl('y')),
            ("ctrl+c", ctrl('c')),
            ("ctrl+x", ctrl('x')),
            ("ctrl+v", ctrl('v')),
            ("ctrl+a", ctrl('a')),
            ("alt+,", alt(',')),
            ("alt+.", alt('.')),
            ("f12", ev(KeyCode::F(12), KeyModifiers::NONE)),
            ("alt+left", ev(KeyCode::Left, KeyModifiers::ALT)),
            ("alt+k", alt('k')),
            ("ctrl+f", ctrl('f')),
            ("ctrl+r", ctrl('r')),
            ("alt+f", alt('f')),
            ("ctrl+p", ctrl('p')),
            ("ctrl+g", ctrl('g')),
            ("ctrl+b", ctrl('b')),
            ("ctrl+e", ctrl('e')),
            ("alt+v", alt('v')),
            ("f6", ev(KeyCode::F(6), KeyModifiers::NONE)),
            ("f5", ev(KeyCode::F(5), KeyModifiers::NONE)),
            ("shift+f5", ev(KeyCode::F(5), KeyModifiers::SHIFT)),
            ("ctrl+f5", ev(KeyCode::F(5), KeyModifiers::CONTROL)),
            ("f4", ev(KeyCode::F(4), KeyModifiers::NONE)),
            ("f8", ev(KeyCode::F(8), KeyModifiers::NONE)),
            ("shift+f8", ev(KeyCode::F(8), KeyModifiers::SHIFT)),
            ("f9", ev(KeyCode::F(9), KeyModifiers::NONE)),
            ("alt+f5", ev(KeyCode::F(5), KeyModifiers::ALT)),
            ("alt+f6", ev(KeyCode::F(6), KeyModifiers::ALT)),
            ("f10", ev(KeyCode::F(10), KeyModifiers::NONE)),
            ("alt+f10", ev(KeyCode::F(10), KeyModifiers::ALT)),
            ("shift+f10", ev(KeyCode::F(10), KeyModifiers::SHIFT)),
            ("alt+/", alt('/')),
        ];
        const DIGITS: [(&str, char); 9] = [
            ("alt+1", '1'),
            ("alt+2", '2'),
            ("alt+3", '3'),
            ("alt+4", '4'),
            ("alt+5", '5'),
            ("alt+6", '6'),
            ("alt+7", '7'),
            ("alt+8", '8'),
            ("alt+9", '9'),
        ];
        all.extend(DIGITS.iter().map(|&(n, c)| (n, alt(c))));
        all
    }

    #[test]
    fn f5_runs_and_f4_toggles_the_run_panel_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::F(5), KeyModifiers::NONE)),
            Input::Action(Action::Run)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::F(4), KeyModifiers::NONE)),
            Input::Action(Action::ToggleRunPanel)
        );
        let map = Keymap::new(&keys(&[("run", one("f9"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::F(9), KeyModifiers::NONE)),
            Input::Action(Action::Run)
        );
    }

    #[test]
    fn shift_f5_stops_and_ctrl_f5_restarts_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::F(5), KeyModifiers::SHIFT)),
            Input::Action(Action::StopRun)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::F(5), KeyModifiers::CONTROL)),
            Input::Action(Action::RestartRun)
        );
    }

    #[test]
    fn f8_and_shift_f8_jump_between_diagnostics_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::F(8), KeyModifiers::NONE)),
            Input::Action(Action::NextDiagnostic)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::F(8), KeyModifiers::SHIFT)),
            Input::Action(Action::PrevDiagnostic)
        );
    }

    #[test]
    fn f9_toggles_a_breakpoint_and_clearing_them_has_no_key() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::F(9), KeyModifiers::NONE)),
            Input::Action(Action::ToggleBreakpoint)
        );
        assert!(Action::commands().any(|a| a == Action::ClearBreakpoints));
        assert_eq!(Action::ToggleBreakpoint.title(), "Toggle breakpoint");
        assert_eq!(Action::ClearBreakpoints.title(), "Clear breakpoints");
    }

    #[test]
    fn alt_f5_starts_debugging_and_alt_f6_stops_it_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::F(5), KeyModifiers::ALT)),
            Input::Action(Action::DebugStart)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::F(6), KeyModifiers::ALT)),
            Input::Action(Action::DebugStop)
        );
        assert_eq!(Action::DebugStart.title(), "Start debugging");
        assert_eq!(Action::DebugStop.title(), "Stop debugging");
        assert!(Action::commands().any(|a| a == Action::DebugStop));
    }

    #[test]
    fn f10_steps_over_alt_f10_into_and_shift_f10_out_by_default() {
        let map = Keymap::default();
        for (modifiers, action, title) in [
            (KeyModifiers::NONE, Action::StepOver, "Step over"),
            (KeyModifiers::ALT, Action::StepInto, "Step into"),
            (KeyModifiers::SHIFT, Action::StepOut, "Step out"),
        ] {
            assert_eq!(
                map.resolve(&ev(KeyCode::F(10), modifiers)),
                Input::Action(action)
            );
            assert_eq!(action.title(), title);
            assert!(Action::commands().any(|a| a == action));
        }
        assert_eq!(map.key_label(Action::StepOut).as_deref(), Some("Shift+F10"));
    }

    #[test]
    fn f12_goes_to_definition_and_alt_left_jumps_back_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::F(12), KeyModifiers::NONE)),
            Input::Action(Action::GoToDefinition)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Left, KeyModifiers::ALT)),
            Input::Action(Action::JumpBack)
        );
    }

    #[test]
    fn alt_slash_completes_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('/'), KeyModifiers::ALT)),
            Input::Action(Action::Complete)
        );
    }

    #[test]
    fn alt_k_hovers_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('k'), KeyModifiers::ALT)),
            Input::Action(Action::Hover)
        );
    }

    #[test]
    fn defaults_build() {
        let map = Keymap::default();
        assert_eq!(
            map.bindings.len()
                + map.tree.len()
                + map.find.len()
                + map.search.len()
                + map.folders.len()
                + map.catalog.len()
                + map.splash.len()
                + map.debug.len(),
            DEFAULT_BINDINGS.len()
        );
    }

    #[test]
    fn commands_are_the_global_actions_but_cancel_with_their_keys() {
        let commands: Vec<Action> = Action::commands().collect();
        assert!(commands.contains(&Action::ToggleSplit));
        assert!(commands.contains(&Action::GoToLine));
        assert!(!commands.contains(&Action::Cancel));
        assert!(!commands.contains(&Action::TreeRename));
        assert!(!commands.contains(&Action::FindNext));
        assert!(!commands.contains(&Action::ProjectReplace));

        let map = Keymap::default();
        assert_eq!(map.key_label(Action::ToggleSplit).as_deref(), Some("Alt+V"));
        assert_eq!(map.key_label(Action::GoToLine).as_deref(), Some("Ctrl+G"));
        assert_eq!(map.key_label(Action::StopRun).as_deref(), Some("Shift+F5"));
        assert_eq!(map.key_label(Action::PrevTab).as_deref(), Some("Alt+,"));
        assert_eq!(
            map.key_label(Action::Select(Motion::WordLeft)).as_deref(),
            Some("Ctrl+Shift+Left")
        );
        // A user's key replaces the default in the label too.
        let map = Keymap::new(&keys(&[("toggle_split", one("ctrl+\\"))])).unwrap();
        assert_eq!(
            map.key_label(Action::ToggleSplit).as_deref(),
            Some("Ctrl+\\")
        );
        let map = Keymap::new(&keys(&[(
            "toggle_split",
            KeyBinding::Many(vec!["f9".into(), "ctrl+alt+s".into()]),
        )]))
        .unwrap();
        assert_eq!(map.key_label(Action::ToggleSplit).as_deref(), Some("F9"));
    }

    #[test]
    fn every_key_in_the_spec_table_parses_and_matches_its_event() {
        for (notation, event) in spec_keys() {
            let config = keys(&[("quit", one(notation))]);
            let map = Keymap::new(&config).unwrap_or_else(|e| panic!("{notation}: {e}"));
            assert_eq!(
                map.resolve(&event),
                Input::Action(Action::Quit),
                "{notation} should match {event:?}"
            );
        }
    }

    #[test]
    fn default_bindings_cover_every_action_once() {
        let mut bound: Vec<Action> = DEFAULT_BINDINGS.iter().map(|&(a, _)| a).collect();
        bound.extend_from_slice(UNBOUND);
        bound.sort_by_key(|a| a.name());
        let mut all = Action::ALL.to_vec();
        all.sort_by_key(|a| a.name());
        assert_eq!(bound, all);
        for &motion in Motion::ALL {
            assert!(Action::ALL.contains(&Action::Move(motion)), "{motion:?}");
            assert!(Action::ALL.contains(&Action::Select(motion)), "{motion:?}");
        }
    }

    #[test]
    fn movement_keys_resolve_to_their_motions() {
        let map = Keymap::default();
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let expected = [
            (KeyCode::Left, none, Motion::Left),
            (KeyCode::Right, none, Motion::Right),
            (KeyCode::Up, none, Motion::Up),
            (KeyCode::Down, none, Motion::Down),
            (KeyCode::Home, none, Motion::LineStart),
            (KeyCode::End, none, Motion::LineEnd),
            (KeyCode::PageUp, none, Motion::PageUp),
            (KeyCode::PageDown, none, Motion::PageDown),
            (KeyCode::Left, ctrl, Motion::WordLeft),
            (KeyCode::Right, ctrl, Motion::WordRight),
            (KeyCode::Home, ctrl, Motion::DocStart),
            (KeyCode::End, ctrl, Motion::DocEnd),
        ];
        assert_eq!(expected.len(), Motion::ALL.len());
        for (code, mods, motion) in expected {
            assert_eq!(
                map.resolve(&ev(code, mods)),
                Input::Action(Action::Move(motion)),
                "{mods:?} {code:?}"
            );
        }
    }

    #[test]
    fn movement_can_be_remapped_by_name() {
        let map = Keymap::new(&keys(&[("word_right", one("alt+right"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::Right, KeyModifiers::ALT)),
            Input::Action(Action::Move(Motion::WordRight))
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Right, KeyModifiers::CONTROL)),
            Input::Ignored
        );
    }

    #[test]
    fn ctrl_q_quits_by_default() {
        let map = Keymap::default();
        let event = ev(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert_eq!(map.resolve(&event), Input::Action(Action::Quit));
    }

    #[test]
    fn ctrl_s_saves_and_esc_cancels_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Input::Action(Action::Save)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Esc, KeyModifiers::NONE)),
            Input::Action(Action::Cancel)
        );
        // Plain letters stay text, so prompts can read their answers from it.
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('s'), KeyModifiers::NONE)),
            Input::Text('s')
        );
    }

    #[test]
    fn remapping_replaces_the_default() {
        let map = Keymap::new(&keys(&[("quit", one("alt+q"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('q'), KeyModifiers::ALT)),
            Input::Action(Action::Quit)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Input::Ignored
        );
    }

    #[test]
    fn a_list_binds_every_key() {
        let binding = KeyBinding::Many(vec!["alt+q".into(), "shift+f9".into()]);
        let map = Keymap::new(&keys(&[("quit", binding)])).unwrap();
        for event in [
            ev(KeyCode::Char('q'), KeyModifiers::ALT),
            ev(KeyCode::F(9), KeyModifiers::SHIFT),
        ] {
            assert_eq!(map.resolve(&event), Input::Action(Action::Quit));
        }
    }

    #[test]
    fn unknown_action_is_an_error() {
        let err = Keymap::new(&keys(&[("fly", one("ctrl+q"))])).unwrap_err();
        assert!(err.to_string().contains("unknown action \"fly\""), "{err}");
    }

    #[test]
    fn unknown_key_or_modifier_is_an_error() {
        for bad in ["ctrl+nope", "hyper+q", "", "ctrl+", "f99", "shift+,"] {
            let result = Keymap::new(&keys(&[("quit", one(bad))]));
            assert!(result.is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn notation_is_case_insensitive_and_allows_plus_key() {
        assert_eq!(parse_key("Ctrl+Q"), parse_key("ctrl+q"));
        let plus = parse_key("ctrl++").unwrap();
        assert_eq!(plus.code, KeyCode::Char('+'));
        assert_eq!(plus.mods, KeyModifiers::CONTROL);
    }

    #[test]
    fn shift_letters_and_backtab_match_however_the_terminal_reports_them() {
        let map = Keymap::new(&keys(&[("quit", one("ctrl+shift+q"))])).unwrap();
        for event in [
            ev(KeyCode::Char('Q'), KeyModifiers::CONTROL),
            ev(
                KeyCode::Char('Q'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
        ] {
            assert_eq!(map.resolve(&event), Input::Action(Action::Quit));
        }
        let map = Keymap::new(&keys(&[("quit", one("shift+tab"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Input::Action(Action::Quit)
        );
    }

    #[test]
    fn plain_characters_are_text() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('q'), KeyModifiers::NONE)),
            Input::Text('q')
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('Q'), KeyModifiers::SHIFT)),
            Input::Text('Q')
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('é'), KeyModifiers::NONE)),
            Input::Text('é')
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            Input::Ignored
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('x'), KeyModifiers::ALT)),
            Input::Ignored
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::F(11), KeyModifiers::NONE)),
            Input::Ignored
        );
    }

    #[test]
    fn editing_keys_resolve_to_their_actions() {
        let map = Keymap::default();
        let none = KeyModifiers::NONE;
        for (code, action) in [
            (KeyCode::Enter, Action::Newline),
            (KeyCode::Backspace, Action::Backspace),
            (KeyCode::Delete, Action::Delete),
            (KeyCode::Tab, Action::Tab),
        ] {
            assert_eq!(
                map.resolve(&ev(code, none)),
                Input::Action(action),
                "{code:?}"
            );
        }
    }

    #[test]
    fn ctrl_z_undoes_and_ctrl_y_redoes_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            Input::Action(Action::Undo)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            Input::Action(Action::Redo)
        );
    }

    #[test]
    fn shift_movement_selects_and_clipboard_keys_resolve() {
        let map = Keymap::default();
        let shift = KeyModifiers::SHIFT;
        let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        let expected = [
            (KeyCode::Left, shift, Motion::Left),
            (KeyCode::Right, shift, Motion::Right),
            (KeyCode::Up, shift, Motion::Up),
            (KeyCode::Down, shift, Motion::Down),
            (KeyCode::Home, shift, Motion::LineStart),
            (KeyCode::End, shift, Motion::LineEnd),
            (KeyCode::PageUp, shift, Motion::PageUp),
            (KeyCode::PageDown, shift, Motion::PageDown),
            (KeyCode::Left, ctrl_shift, Motion::WordLeft),
            (KeyCode::Right, ctrl_shift, Motion::WordRight),
            (KeyCode::Home, ctrl_shift, Motion::DocStart),
            (KeyCode::End, ctrl_shift, Motion::DocEnd),
        ];
        assert_eq!(expected.len(), Motion::ALL.len());
        for (code, mods, motion) in expected {
            assert_eq!(
                map.resolve(&ev(code, mods)),
                Input::Action(Action::Select(motion)),
                "{mods:?} {code:?}"
            );
        }
        let ctrl = KeyModifiers::CONTROL;
        for (c, action) in [
            ('a', Action::SelectAll),
            ('c', Action::Copy),
            ('x', Action::Cut),
            ('v', Action::Paste),
        ] {
            assert_eq!(
                map.resolve(&ev(KeyCode::Char(c), ctrl)),
                Input::Action(action),
                "ctrl+{c}"
            );
        }
    }

    #[test]
    fn a_burst_with_a_line_break_is_a_paste() {
        let key = |code| Event::Key(ev(code, KeyModifiers::NONE));
        let mut release = ev(KeyCode::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        let pasted = [
            key(KeyCode::Char('a')),
            Event::Key(release),
            key(KeyCode::Enter),
            key(KeyCode::Tab),
            Event::Key(ev(KeyCode::Char('B'), KeyModifiers::SHIFT)),
        ];
        assert_eq!(burst_as_paste(&pasted).as_deref(), Some("a\n\tB"));

        // Plain letters stay typing, and so does a lone Enter.
        let typed = [key(KeyCode::Char('a')), key(KeyCode::Char('b'))];
        assert_eq!(burst_as_paste(&typed), None);
        assert_eq!(burst_as_paste(&[key(KeyCode::Enter)]), None);
        // A shortcut or a non-text key in the burst means it wasn't a paste.
        let shortcut = [
            key(KeyCode::Char('a')),
            key(KeyCode::Enter),
            Event::Key(ev(KeyCode::Char('z'), KeyModifiers::CONTROL)),
        ];
        assert_eq!(burst_as_paste(&shortcut), None);
        let arrow = [key(KeyCode::Enter), key(KeyCode::Left)];
        assert_eq!(burst_as_paste(&arrow), None);
        assert_eq!(
            burst_as_paste(&[key(KeyCode::Enter), Event::FocusLost]),
            None
        );
    }

    #[test]
    fn ctrl_b_toggles_and_ctrl_e_focuses_the_tree_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('b'), KeyModifiers::CONTROL)),
            Input::Action(Action::ToggleTree)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('e'), KeyModifiers::CONTROL)),
            Input::Action(Action::FocusTree)
        );
        let map = Keymap::new(&keys(&[("toggle_tree", one("alt+b"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('b'), KeyModifiers::ALT)),
            Input::Action(Action::ToggleTree)
        );
    }

    #[test]
    fn tree_letters_are_actions_only_in_tree_scope() {
        let map = Keymap::default();
        let none = KeyModifiers::NONE;
        for (event, action) in [
            (ev(KeyCode::Char('a'), none), Action::TreeNewFile),
            (
                ev(KeyCode::Char('A'), KeyModifiers::SHIFT),
                Action::TreeNewFolder,
            ),
            (ev(KeyCode::Char('A'), none), Action::TreeNewFolder),
            (ev(KeyCode::Char('r'), none), Action::TreeRename),
            (ev(KeyCode::Char('d'), none), Action::TreeDelete),
        ] {
            assert_eq!(map.resolve_in(&event, Scope::Tree), Input::Action(action));
            assert!(matches!(map.resolve(&event), Input::Text(_)), "{event:?}");
        }
        // Global bindings still work in the tree.
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Char('q'), KeyModifiers::CONTROL), Scope::Tree),
            Input::Action(Action::Quit)
        );
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Char('x'), none), Scope::Tree),
            Input::Text('x')
        );

        let map = Keymap::new(&keys(&[("tree_delete", one("delete"))])).unwrap();
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Delete, none), Scope::Tree),
            Input::Action(Action::TreeDelete)
        );
        // Outside the tree Delete keeps deleting text.
        assert_eq!(
            map.resolve(&ev(KeyCode::Delete, none)),
            Input::Action(Action::Delete)
        );
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Char('d'), none), Scope::Tree),
            Input::Text('d')
        );
    }

    #[test]
    fn splash_keys_are_actions_only_in_splash_scope() {
        let map = Keymap::default();
        let none = KeyModifiers::NONE;
        for (event, action) in [
            (ev(KeyCode::Up, none), Action::SplashUp),
            (ev(KeyCode::Down, none), Action::SplashDown),
            (ev(KeyCode::Enter, none), Action::SplashRun),
            (ev(KeyCode::Char('n'), none), Action::SplashNewFile),
            (ev(KeyCode::Char('d'), none), Action::SplashNewDirectory),
            (ev(KeyCode::Char('o'), none), Action::SplashOpenDirectory),
            (ev(KeyCode::Esc, none), Action::SplashDismiss),
            (ev(KeyCode::Char('q'), none), Action::SplashQuit),
        ] {
            assert_eq!(map.resolve_in(&event, Scope::Splash), Input::Action(action));
            assert_ne!(map.resolve(&event), Input::Action(action), "{event:?}");
        }
        // Global keys still reach the app from the splash: Ctrl+N leaves it,
        // Ctrl+P and Ctrl+E work as anywhere.
        assert_eq!(
            map.resolve_in(
                &ev(KeyCode::Char('n'), KeyModifiers::CONTROL),
                Scope::Splash
            ),
            Input::Action(Action::NewFile)
        );
        assert_eq!(
            map.resolve_in(
                &ev(KeyCode::Char('e'), KeyModifiers::CONTROL),
                Scope::Splash
            ),
            Input::Action(Action::FocusTree)
        );
        // The splash's actions aren't commands in the cast palette.
        assert!(!Action::commands().any(|a| a.scope() == Scope::Splash));

        let map = Keymap::new(&keys(&[("splash_new_file", one("f"))])).unwrap();
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Char('f'), none), Scope::Splash),
            Input::Action(Action::SplashNewFile)
        );
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Char('n'), none), Scope::Splash),
            Input::Text('n')
        );
    }

    #[test]
    fn debug_panel_keys_are_actions_only_in_debug_scope() {
        let map = Keymap::default();
        let none = KeyModifiers::NONE;
        for (event, action) in [
            (ev(KeyCode::Up, none), Action::DebugUp),
            (ev(KeyCode::Down, none), Action::DebugDown),
            (ev(KeyCode::Enter, none), Action::DebugActivate),
            (ev(KeyCode::Right, none), Action::DebugExpand),
            (ev(KeyCode::Left, none), Action::DebugCollapse),
            (ev(KeyCode::Tab, none), Action::DebugSwitchPane),
        ] {
            assert_eq!(map.resolve_in(&event, Scope::Debug), Input::Action(action));
            assert_ne!(map.resolve(&event), Input::Action(action), "{event:?}");
        }
        // F6 and Esc still leave the panel; Alt+F6 still stops debugging.
        assert_eq!(
            map.resolve_in(&ev(KeyCode::F(6), none), Scope::Debug),
            Input::Action(Action::CycleFocus)
        );
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Esc, none), Scope::Debug),
            Input::Action(Action::Cancel)
        );
        assert!(!Action::commands().any(|a| a.scope() == Scope::Debug));

        let map = Keymap::new(&keys(&[("debug_expand", one("l"))])).unwrap();
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Char('l'), none), Scope::Debug),
            Input::Action(Action::DebugExpand)
        );
    }

    #[test]
    fn tab_keys_resolve_to_their_actions_by_default() {
        let map = Keymap::default();
        let ctrl = |c| ev(KeyCode::Char(c), KeyModifiers::CONTROL);
        let alt = |c| ev(KeyCode::Char(c), KeyModifiers::ALT);
        let mut expected = vec![
            (alt(','), Action::PrevTab),
            (alt('.'), Action::NextTab),
            (ctrl('w'), Action::CloseTab),
            (ctrl('n'), Action::NewFile),
            (alt('s'), Action::SaveAs),
        ];
        for n in 1..=9u8 {
            expected.push((alt(char::from(b'0' + n)), Action::GoToTab(n)));
        }
        for (event, action) in expected {
            assert_eq!(map.resolve(&event), Input::Action(action), "{event:?}");
        }
        assert_eq!(Action::from_name("tab_9"), Some(Action::GoToTab(9)));
        let map = Keymap::new(&keys(&[("tab_3", one("ctrl+3"))])).unwrap();
        assert_eq!(map.resolve(&ctrl('3')), Input::Action(Action::GoToTab(3)));
        assert_eq!(map.resolve(&alt('3')), Input::Ignored);
    }

    #[test]
    fn split_keys_resolve_to_their_actions_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('v'), KeyModifiers::ALT)),
            Input::Action(Action::ToggleSplit)
        );
        assert_eq!(
            map.resolve(&ev(KeyCode::F(6), KeyModifiers::NONE)),
            Input::Action(Action::CycleFocus)
        );
        // Both work from the tree too, and can be remapped by name.
        assert_eq!(
            map.resolve_in(&ev(KeyCode::F(6), KeyModifiers::NONE), Scope::Tree),
            Input::Action(Action::CycleFocus)
        );
        let map = Keymap::new(&keys(&[("cycle_focus", one("f7"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::F(7), KeyModifiers::NONE)),
            Input::Action(Action::CycleFocus)
        );
        assert_eq!(Action::from_name("toggle_split"), Some(Action::ToggleSplit));
    }

    #[test]
    fn picker_keys_resolve_to_their_actions_by_default() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('p'), KeyModifiers::CONTROL)),
            Input::Action(Action::GoToFile)
        );
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Char('g'), KeyModifiers::CONTROL), Scope::Tree),
            Input::Action(Action::GoToLine)
        );
        let map = Keymap::new(&keys(&[("go_to_file", one("alt+p"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('p'), KeyModifiers::ALT)),
            Input::Action(Action::GoToFile)
        );
        assert_eq!(Action::from_name("go_to_line"), Some(Action::GoToLine));
    }

    #[test]
    fn find_keys_resolve_and_bar_keys_only_count_in_find_scope() {
        let map = Keymap::default();
        let none = KeyModifiers::NONE;
        let alt = |c| ev(KeyCode::Char(c), KeyModifiers::ALT);
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('f'), KeyModifiers::CONTROL)),
            Input::Action(Action::Find)
        );
        for (event, action) in [
            (ev(KeyCode::Enter, none), Action::FindNext),
            (ev(KeyCode::Enter, KeyModifiers::SHIFT), Action::FindPrev),
            (alt('c'), Action::FindCase),
            (alt('r'), Action::FindRegex),
        ] {
            assert_eq!(map.resolve_in(&event, Scope::Find), Input::Action(action));
        }
        // Outside the bar Enter is still a newline and the Alt keys do nothing.
        assert_eq!(
            map.resolve(&ev(KeyCode::Enter, none)),
            Input::Action(Action::Newline)
        );
        assert_eq!(map.resolve(&alt('c')), Input::Ignored);
        // Global keys still reach the bar.
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Esc, none), Scope::Find),
            Input::Action(Action::Cancel)
        );
        let map = Keymap::new(&keys(&[("find_case", one("alt+i"))])).unwrap();
        assert_eq!(
            map.resolve_in(&alt('i'), Scope::Find),
            Input::Action(Action::FindCase)
        );
        assert_eq!(map.resolve_in(&alt('c'), Scope::Find), Input::Ignored);
    }

    #[test]
    fn replace_opens_globally_and_replace_all_only_counts_in_the_find_bar() {
        let map = Keymap::default();
        let alt_a = ev(KeyCode::Char('a'), KeyModifiers::ALT);
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Input::Action(Action::Replace)
        );
        assert_eq!(
            map.resolve_in(&alt_a, Scope::Find),
            Input::Action(Action::ReplaceAll)
        );
        assert_eq!(map.resolve(&alt_a), Input::Ignored);
        let map = Keymap::new(&keys(&[("replace_all", one("alt+shift+a"))])).unwrap();
        assert_eq!(map.resolve_in(&alt_a, Scope::Find), Input::Ignored);
    }

    #[test]
    fn alt_f_opens_project_search_and_can_be_remapped() {
        let map = Keymap::default();
        let alt_f = ev(KeyCode::Char('f'), KeyModifiers::ALT);
        assert_eq!(map.resolve(&alt_f), Input::Action(Action::ProjectSearch));
        assert_eq!(
            Action::from_name("project_search"),
            Some(Action::ProjectSearch)
        );
        let map = Keymap::new(&keys(&[("project_search", one("alt+g"))])).unwrap();
        assert_eq!(map.resolve(&alt_f), Input::Ignored);
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('g'), KeyModifiers::ALT)),
            Input::Action(Action::ProjectSearch)
        );
    }

    #[test]
    fn alt_a_replaces_across_the_project_in_search_and_in_the_file_in_the_find_bar() {
        let map = Keymap::default();
        let alt_a = ev(KeyCode::Char('a'), KeyModifiers::ALT);
        assert_eq!(
            map.resolve_in(&alt_a, Scope::Search),
            Input::Action(Action::ProjectReplace)
        );
        assert_eq!(
            map.resolve_in(&alt_a, Scope::Find),
            Input::Action(Action::ReplaceAll)
        );
        assert_eq!(map.resolve(&alt_a), Input::Ignored);
        // Alt+Enter no longer replaces anything.
        let alt_enter = ev(KeyCode::Enter, KeyModifiers::ALT);
        assert_ne!(
            map.resolve_in(&alt_enter, Scope::Search),
            Input::Action(Action::ProjectReplace)
        );
        // Project search still reads the find bar's keys.
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Char('c'), KeyModifiers::ALT), Scope::Search),
            Input::Action(Action::FindCase)
        );
        assert_eq!(
            Action::from_name("project_replace"),
            Some(Action::ProjectReplace)
        );
        let map = Keymap::new(&keys(&[("project_replace", one("alt+enter"))])).unwrap();
        assert_eq!(
            map.resolve_in(&alt_enter, Scope::Search),
            Input::Action(Action::ProjectReplace)
        );
        assert_eq!(
            map.resolve_in(&alt_a, Scope::Search),
            Input::Action(Action::ReplaceAll)
        );
    }

    #[test]
    fn open_directory_is_a_command_with_no_key_until_one_is_bound() {
        let map = Keymap::default();
        assert!(Action::commands().any(|a| a == Action::OpenDirectory));
        assert_eq!(Action::OpenDirectory.title(), "Open directory");
        assert_eq!(map.key_label(Action::OpenDirectory), None);
        let map = Keymap::new(&keys(&[("open_directory", one("alt+o"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('o'), KeyModifiers::ALT)),
            Input::Action(Action::OpenDirectory)
        );
    }

    #[test]
    fn language_servers_is_a_command_with_no_key_until_one_is_bound() {
        let map = Keymap::default();
        assert!(Action::commands().any(|a| a == Action::LanguageServers));
        assert_eq!(Action::LanguageServers.title(), "Language servers");
        assert_eq!(Action::LanguageServers.scope(), Scope::Global);
        assert_eq!(map.key_label(Action::LanguageServers), None);
        let map = Keymap::new(&keys(&[("language_servers", one("alt+l"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('l'), KeyModifiers::ALT)),
            Input::Action(Action::LanguageServers)
        );
    }

    #[test]
    fn settings_is_a_command_with_no_key_until_one_is_bound() {
        let map = Keymap::default();
        assert!(Action::commands().any(|a| a == Action::Settings));
        assert_eq!(Action::Settings.title(), "Settings");
        assert_eq!(Action::Settings.scope(), Scope::Global);
        assert_eq!(map.key_label(Action::Settings), None);
        let map = Keymap::new(&keys(&[("settings", one("alt+j"))])).unwrap();
        assert_eq!(
            map.resolve(&ev(KeyCode::Char('j'), KeyModifiers::ALT)),
            Input::Action(Action::Settings)
        );
    }

    #[test]
    fn c_copies_only_in_the_catalog() {
        let map = Keymap::default();
        let c = ev(KeyCode::Char('c'), KeyModifiers::NONE);
        assert_eq!(
            map.resolve_in(&c, Scope::Catalog),
            Input::Action(Action::CatalogCopy)
        );
        assert_eq!(map.resolve(&c), Input::Text('c'));
        assert!(!Action::commands().any(|a| a == Action::CatalogCopy));
        // The arrows and Esc still mean what they do globally.
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Down, KeyModifiers::NONE), Scope::Catalog),
            Input::Action(Action::Move(Motion::Down))
        );
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Esc, KeyModifiers::NONE), Scope::Catalog),
            Input::Action(Action::Cancel)
        );
    }

    #[test]
    fn ctrl_enter_opens_the_shown_folder_only_in_the_folder_browser() {
        let map = Keymap::default();
        let ctrl_enter = ev(KeyCode::Enter, KeyModifiers::CONTROL);
        assert_eq!(
            map.resolve_in(&ctrl_enter, Scope::Folders),
            Input::Action(Action::OpenFolderHere)
        );
        assert_eq!(map.resolve(&ctrl_enter), Input::Ignored);
        assert!(!Action::commands().any(|a| a == Action::OpenFolderHere));
        // Everything else still means what it does globally.
        assert_eq!(
            map.resolve_in(&ev(KeyCode::Enter, KeyModifiers::NONE), Scope::Folders),
            Input::Action(Action::Newline)
        );
    }

    #[test]
    fn key_release_is_ignored() {
        let map = Keymap::default();
        let mut release = ev(KeyCode::Char('q'), KeyModifiers::CONTROL);
        release.kind = KeyEventKind::Release;
        assert_eq!(map.resolve(&release), Input::Ignored);
    }

    #[test]
    fn key_event_helper_round_trips() {
        let map = Keymap::default();
        assert_eq!(
            map.resolve(&key_event("ctrl+q")),
            Input::Action(Action::Quit)
        );
        assert_eq!(map.resolve(&key_event("q")), Input::Text('q'));
    }
}
