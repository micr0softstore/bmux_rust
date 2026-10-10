//! Regression tests for event delivery across egui's discarded sizing passes.
use super::*;

fn fixture(directory: &std::path::Path, ctx: &egui::Context) -> BmuxApp {
    BmuxApp::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        Store::at(directory).unwrap(),
    )
}

fn shortcut(key: Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        },
    }
}

#[test]
fn multipass_shortcut_survives_discard_and_splits_exactly_once() {
    let directory = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = fixture(directory.path(), &ctx);
    let pane = app.focus().unwrap();
    let result = ctx.run(
        egui::RawInput {
            events: vec![shortcut(Key::D)],
            ..Default::default()
        },
        |ctx| {
            app.keyboard(ctx);
            assert!(
                ctx.input(|input| input.events.is_empty()),
                "Shortcut leaked into terminal input"
            );
            if ctx.current_pass_index() == 0 {
                ctx.request_discard("Test a sizing pass after the shortcut was captured");
                app.commit_frame_actions(ctx);
                assert_eq!(
                    app.tab().unwrap().root.panes().len(),
                    1,
                    "Discarded layout performed the split"
                );
            } else {
                // A widget can report the same logical action again while laying out.
                app.actions
                    .push(Action::Split(pane, SplitDirection::SideBySide));
                app.commit_frame_actions(ctx);
                assert_eq!(app.tab().unwrap().root.panes().len(), 2);
            }
        },
    );
    assert_eq!(result.platform_output.num_completed_passes, 2);
    assert_eq!(app.tab().unwrap().root.panes().len(), 2);
}

#[test]
fn action_first_reported_in_later_pass_is_committed() {
    let directory = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = fixture(directory.path(), &ctx);
    let before = app.workspace().unwrap().tabs.len();
    let result = ctx.run(Default::default(), |ctx| {
        if ctx.current_pass_index() == 0 {
            ctx.request_discard("Navigator size became known");
        } else {
            app.actions.push(Action::NewTab);
        }
        app.commit_frame_actions(ctx);
    });
    assert_eq!(result.platform_output.num_completed_passes, 2);
    assert_eq!(app.workspace().unwrap().tabs.len(), before + 1);
}

#[test]
fn repeated_real_shortcuts_in_one_pass_are_not_collapsed() {
    let directory = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = fixture(directory.path(), &ctx);
    let before = app.workspace().unwrap().tabs.len();
    let _ = ctx.run(
        egui::RawInput {
            events: vec![shortcut(Key::T), shortcut(Key::T)],
            ..Default::default()
        },
        |ctx| {
            app.keyboard(ctx);
            if ctx.current_pass_index() == 0 {
                ctx.request_discard("Sizing");
            }
            app.commit_frame_actions(ctx);
        },
    );
    assert_eq!(app.workspace().unwrap().tabs.len(), before + 2);
}

#[test]
fn same_action_works_in_consecutive_frames_and_late_discard_does_not_duplicate_it() {
    let directory = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = fixture(directory.path(), &ctx);
    let before = app.workspace().unwrap().tabs.len();
    for _ in 0..2 {
        let _ = ctx.run(Default::default(), |ctx| {
            app.actions.push(Action::NewTab);
            app.commit_frame_actions(ctx);
            if ctx.current_pass_index() == 0 {
                // Simulate an external plugin requesting another pass after our UI.
                ctx.request_discard("Late plugin request");
            }
        });
    }
    assert_eq!(app.workspace().unwrap().tabs.len(), before + 2);
}

#[test]
fn exhausted_discard_budget_still_commits_visible_pass_actions() {
    let directory = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
    let mut app = fixture(directory.path(), &ctx);
    let before = app.workspace().unwrap().tabs.len();
    let result = ctx.run(Default::default(), |ctx| {
        app.actions.push(Action::NewTab);
        ctx.request_discard("Budget is exhausted; this pass remains visible");
        app.commit_frame_actions(ctx);
    });
    assert_eq!(result.platform_output.num_completed_passes, 1);
    assert_eq!(app.workspace().unwrap().tabs.len(), before + 1);
}
