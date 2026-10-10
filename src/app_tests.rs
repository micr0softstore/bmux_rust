//! Native-window-free interaction checks for workspace actions and shortcuts.
use super::*;

fn fixture(directory: &std::path::Path) -> BmuxApp {
    BmuxApp {
        state: State::default(),
        settings: Settings::default(),
        store: Store::at(directory).unwrap(),
        hosts: HashMap::new(),
        settings_open: false,
        form: WorkspaceForm::default(),
        rename: None,
        history: None,
        errors: Vec::new(),
        actions: Vec::new(),
        last_save: Instant::now(),
        last_state: String::new(),
        last_settings: String::new(),
        font_path: String::new(),
        zoom: 0.0,
        dragging_workspace: None,
        last_metadata: Instant::now(),
        corner_mask: None,
    }
}

fn press(app: &mut BmuxApp, key: Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    let ctx = egui::Context::default();
    let mut remaining = Vec::new();
    let _ = ctx.run(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        },
        |ctx| {
            app.keyboard(ctx);
            remaining = ctx.input(|input| input.events.clone());
        },
    );
    app.process_actions();
    remaining
}

#[test]
fn terminal_control_c_is_preserved_and_app_shortcut_is_consumed() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    let pane = app.focus().unwrap();
    let remaining = press(&mut app, Key::C, egui::Modifiers::CTRL);
    assert_eq!(remaining.len(), 1);
    assert_eq!(app.tab().unwrap().root.panes().len(), 1);
    let remaining = press(
        &mut app,
        Key::D,
        egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        },
    );
    assert!(remaining.is_empty());
    assert_eq!(app.tab().unwrap().root.panes().len(), 2);
    assert_ne!(app.focus().unwrap(), pane);
    assert!(app.tab().unwrap().root.pane(pane).is_some());
}

#[test]
fn settings_window_prevents_application_shortcuts() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    app.settings_open = true;
    let remaining = press(
        &mut app,
        Key::W,
        egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(remaining.len(), 1);
    assert!(app.actions.is_empty());
}

#[test]
fn split_close_tab_switch_and_restore_keep_stable_identities() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    let workspace = app.state.selected;
    let first_tab = app.tab().unwrap().id;
    let first_pane = app.focus().unwrap();
    app.actions
        .push(Action::Split(first_pane, SplitDirection::SideBySide));
    app.process_actions();
    let sibling = app.focus().unwrap();
    app.actions
        .push(Action::Split(sibling, SplitDirection::Stacked));
    app.process_actions();
    let nested = app.focus().unwrap();
    app.actions.push(Action::ClosePane(sibling));
    app.process_actions();
    assert_eq!(app.tab().unwrap().root.panes().len(), 2);
    assert!(app.tab().unwrap().root.pane(first_pane).is_some());
    assert!(app.tab().unwrap().root.pane(nested).is_some());
    app.actions.push(Action::NewTab);
    app.process_actions();
    assert_ne!(app.tab().unwrap().id, first_tab);
    app.actions.push(Action::SelectTab(first_tab));
    app.process_actions();
    assert_eq!(app.focus(), Some(nested));
    app.store.save_state(&app.state).unwrap();
    let restored = app.store.load_state().unwrap();
    assert_eq!(restored.selected, workspace);
    assert_eq!(restored.active_tab().unwrap().id, first_tab);
    assert_eq!(restored.active_pane().unwrap().id, nested);
}

#[test]
fn closing_last_terminal_replaces_it_and_deleting_last_workspace_stays_empty() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    let old_tab = app.tab().unwrap().id;
    let old_pane = app.focus().unwrap();
    app.actions.push(Action::ClosePane(old_pane));
    app.process_actions();
    app.process_actions();
    assert_ne!(app.tab().unwrap().id, old_tab);
    assert_ne!(app.focus(), Some(old_pane));
    assert_eq!(app.workspace().unwrap().tabs.len(), 1);
    for id in app
        .state
        .workspaces
        .iter()
        .map(|workspace| workspace.id)
        .collect::<Vec<_>>()
    {
        app.actions.push(Action::DeleteWorkspace(id));
    }
    app.process_actions();
    assert!(app.state.workspaces.is_empty());
    app.store.save_state(&app.state).unwrap();
    assert!(app.store.load_state().unwrap().workspaces.is_empty());
}

#[test]
fn duplicate_workspace_gets_new_session_ids_and_keeps_connection_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    let workspace = Workspace::ssh("Production", "ssh -p 2202 deploy@example.test", "prod");
    let original = workspace.id;
    let old_pane = workspace.panes()[0].id;
    app.state.add_workspace(workspace);
    app.actions.push(Action::DuplicateWorkspace(original));
    app.process_actions();
    let duplicate = app.workspace().unwrap();
    assert_ne!(duplicate.id, original);
    assert_ne!(duplicate.panes()[0].id, old_pane);
    assert_eq!(duplicate.ssh_command, "ssh -p 2202 deploy@example.test");
    assert_eq!(duplicate.ssh_tmux_session, "prod");
    assert_eq!(duplicate.kind, WorkspaceKind::Ssh);
}

fn render_terminal_ui(
    app: &mut BmuxApp,
    ctx: &egui::Context,
    width: f32,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let colors = app.settings.colors(true);
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 500.0))),
            events,
            ..Default::default()
        },
        |ctx| app.terminal_area(ctx, &colors),
    );
    app.process_actions();
    output
}

fn painted_text(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn collect(shape: &egui::Shape, clip: Rect, texts: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(text) => {
                let rect = text
                    .galley
                    .rect
                    .translate(text.pos.to_vec2())
                    .intersect(clip);
                if rect.is_positive() {
                    texts.push((text.galley.job.text.clone(), rect));
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, clip, texts);
                }
            }
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for shape in &output.shapes {
        collect(&shape.shape, shape.clip_rect, &mut texts);
    }
    texts
}

fn click_terminal_ui(app: &mut BmuxApp, ctx: &egui::Context, width: f32, position: Pos2) {
    for pressed in [true, false] {
        render_terminal_ui(
            app,
            ctx,
            width,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

#[test]
fn navigator_remains_visible_without_hover_and_reserves_terminal_space() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    let ctx = egui::Context::default();
    let tab = app.tab().unwrap().id;
    render_terminal_ui(&mut app, &ctx, 800.0, Vec::new());
    let original = ctx.read_response(Id::new(("tab", tab))).unwrap().rect;
    let output = render_terminal_ui(
        &mut app,
        &ctx,
        800.0,
        vec![egui::Event::PointerMoved(Pos2::new(400.0, 300.0))],
    );
    assert!(painted_text(&output).iter().any(|(text, _)| text == ">_ 1"));
    render_terminal_ui(&mut app, &ctx, 800.0, vec![egui::Event::PointerGone]);
    let output = render_terminal_ui(&mut app, &ctx, 800.0, Vec::new());
    assert!(painted_text(&output).iter().any(|(text, _)| text == ">_ 1"));
    let current = ctx.read_response(Id::new(("tab", tab))).unwrap().rect;
    assert_eq!(
        original, current,
        "hover must not move the navigation or PTY grid"
    );
    let terminal_background = output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::Shape::Rect(rect) = &shape.shape {
                (rect.rect.bottom() == 500.0 && rect.rect.width() == 800.0 && rect.fill.a() > 0)
                    .then_some(rect.rect)
            } else {
                None
            }
        })
        .expect("the terminal surface is painted independently of the navigator");
    assert!(terminal_background.top() >= current.bottom());
    assert_eq!(terminal_background.top(), 42.0);
}

#[test]
fn split_count_and_pane_picker_follow_the_active_tree_and_focus_clicked_pane() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    let ctx = egui::Context::default();
    let first = app.focus().unwrap();
    app.actions
        .push(Action::Split(first, SplitDirection::SideBySide));
    app.process_actions();
    let second = app.focus().unwrap();
    assert_ne!(first, second);
    assert_eq!(app.workspace().unwrap().tabs.len(), 1);
    render_terminal_ui(&mut app, &ctx, 800.0, Vec::new());
    let output = render_terminal_ui(&mut app, &ctx, 800.0, Vec::new());
    let texts = painted_text(&output);
    assert!(texts.iter().any(|(text, _)| text == ">_ 1 · 2 panes"));
    let picker = texts
        .iter()
        .find(|(text, _)| text == "2 panes")
        .unwrap()
        .1
        .center();
    click_terminal_ui(&mut app, &ctx, 800.0, picker);
    render_terminal_ui(&mut app, &ctx, 800.0, Vec::new());
    let output = render_terminal_ui(&mut app, &ctx, 800.0, Vec::new());
    let texts = painted_text(&output);
    assert!(texts.iter().any(|(text, _)| text.starts_with("Pane 2")));
    let first_row = texts
        .iter()
        .find(|(text, _)| text.starts_with("Pane 1"))
        .unwrap()
        .1
        .center();
    click_terminal_ui(&mut app, &ctx, 800.0, first_row);
    assert_eq!(app.focus(), Some(first));
    assert_eq!(app.tab().unwrap().root.panes().len(), 2);
    app.actions.push(Action::ClosePane(second));
    app.process_actions();
    let output = render_terminal_ui(&mut app, &ctx, 800.0, Vec::new());
    assert!(
        !painted_text(&output)
            .iter()
            .any(|(text, _)| text.contains("2 panes"))
    );
}

#[test]
fn overflow_navigator_reveals_new_and_keyboard_selected_tabs_after_resizing() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = fixture(temp.path());
    let ctx = egui::Context::default();
    let first = app.tab().unwrap().id;
    for _ in 0..15 {
        app.actions.push(Action::NewTab);
    }
    app.process_actions();
    let last = app.tab().unwrap().id;
    for width in [1000.0, 420.0, 220.0, 850.0] {
        for _ in 0..3 {
            render_terminal_ui(&mut app, &ctx, width, Vec::new());
        }
        let response = ctx.read_response(Id::new(("tab", last))).unwrap();
        assert!(
            response.interact_rect.width() > 30.0,
            "active tab must remain clickable at width {width}"
        );
        assert!(response.interact_rect.left() >= 0.0 && response.interact_rect.right() <= width);
        assert!(response.rect.bottom() <= 42.0);
    }
    app.actions.push(Action::SelectTab(first));
    app.process_actions();
    for _ in 0..3 {
        render_terminal_ui(&mut app, &ctx, 420.0, Vec::new());
    }
    let first_response = ctx.read_response(Id::new(("tab", first))).unwrap();
    assert!(first_response.interact_rect.width() > 30.0);
    click_terminal_ui(&mut app, &ctx, 420.0, first_response.interact_rect.center());
    assert_eq!(app.tab().unwrap().id, first);
}

fn render_split_layout(
    app: &mut BmuxApp,
    ctx: &egui::Context,
    scale: f32,
    rect: Rect,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Color32) {
    ctx.set_pixels_per_point(scale);
    let mut colors = app.settings.colors(true);
    colors.background = colors.background.gamma_multiply(0.4);
    let style = TerminalStyle {
        background: colors.background,
        foreground: colors.foreground,
        ..Default::default()
    };
    let focus = app.focus().unwrap();
    let mut root = app.tab().unwrap().root.clone();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(650.0, 450.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    app.split_ui(ui, &mut root, rect, focus, &style, &colors, true);
                });
        },
    );
    app.tab_mut().unwrap().root = root;
    (output, colors.background)
}

#[test]
fn nested_split_separators_are_opaque_and_cover_their_gaps_at_fractional_dpi() {
    for scale in [1.0, 1.25, 2.0] {
        for direction in [SplitDirection::SideBySide, SplitDirection::Stacked] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = fixture(directory.path());
            let ctx = egui::Context::default();
            let original = app.focus().unwrap();
            app.actions.push(Action::Split(original, direction));
            app.process_actions();
            let sibling = app.focus().unwrap();
            let opposite = if direction == SplitDirection::SideBySide {
                SplitDirection::Stacked
            } else {
                SplitDirection::SideBySide
            };
            app.actions.push(Action::Split(original, opposite));
            app.actions.push(Action::Split(sibling, opposite));
            app.process_actions();
            if let SplitNode::Split {
                ratio,
                first,
                second,
                ..
            } = &mut app.tab_mut().unwrap().root
            {
                *ratio = 0.317;
                if let SplitNode::Split { ratio, .. } = first.as_mut() {
                    *ratio = 0.413;
                }
                if let SplitNode::Split { ratio, .. } = second.as_mut() {
                    *ratio = 0.671;
                }
            }
            let rect = Rect::from_min_size(Pos2::new(8.0, 8.0), Vec2::new(600.0, 400.0));
            let (output, pane_color) = render_split_layout(&mut app, &ctx, scale, rect, Vec::new());
            assert_eq!(output.pixels_per_point, scale);
            let mut partitions = Vec::new();
            let mut separator_count = 0;
            let mut pane_count = 0;
            for clipped in &output.shapes {
                match &clipped.shape {
                    egui::Shape::Mesh(mesh) => {
                        separator_count += 1;
                        assert!(
                            mesh.vertices.iter().all(|vertex| vertex.color.a() == 255),
                            "separator must remain opaque with a translucent terminal theme"
                        );
                        for vertex in &mesh.vertices {
                            for position in [vertex.pos.x, vertex.pos.y] {
                                assert!(
                                    (position * scale - (position * scale).round()).abs() < 0.001,
                                    "divider edge must land on a physical pixel at scale {scale}"
                                );
                            }
                        }
                        partitions.push(mesh.calc_bounds());
                        // Inspect the real tessellated primitives as well: anti-aliased
                        // line/rect fringes must not reintroduce transparent vertices.
                        for primitive in ctx.tessellate(vec![clipped.clone()], scale) {
                            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                                assert!(mesh.vertices.iter().all(|vertex| vertex.color.a() == 255));
                            }
                        }
                    }
                    egui::Shape::Rect(shape) if shape.fill == pane_color => {
                        pane_count += 1;
                        assert!(shape.fill.a() < 255, "only the separators become opaque");
                        partitions.push(shape.rect);
                    }
                    _ => {}
                }
            }
            assert_eq!(separator_count, 3);
            assert_eq!(pane_count, 4);
            for (index, partition) in partitions.iter().enumerate() {
                assert!(rect.contains_rect(*partition));
                assert!(partition.is_positive());
                for other in &partitions[index + 1..] {
                    let overlap = partition.intersect(*other);
                    assert!(
                        overlap.width() <= 0.001 || overlap.height() <= 0.001,
                        "pane/divider geometry must not overlap at nested junctions"
                    );
                }
            }
            let covered_area: f32 = partitions.iter().map(|partition| partition.area()).sum();
            assert!(
                (covered_area - rect.area()).abs() < 0.1,
                "every point must belong to a pane or an opaque divider; no uncovered seam"
            );
        }
    }
}

#[test]
fn tiny_nested_splits_never_produce_inverted_or_nonfinite_rectangles() {
    for scale in [1.0, 1.25, 2.0] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = fixture(directory.path());
        let ctx = egui::Context::default();
        for index in 0..8 {
            let pane = app.focus().unwrap();
            app.actions.push(Action::Split(
                pane,
                if index % 2 == 0 {
                    SplitDirection::SideBySide
                } else {
                    SplitDirection::Stacked
                },
            ));
            app.process_actions();
        }
        let rect = Rect::from_min_size(Pos2::new(8.1, 8.3), Vec2::new(1.1, 1.3));
        let (output, _) = render_split_layout(&mut app, &ctx, scale, rect, Vec::new());
        for shape in output.shapes {
            if let egui::Shape::Mesh(mesh) = shape.shape {
                let bounds = mesh.calc_bounds();
                assert!(bounds.is_positive() && bounds.is_finite());
                assert!(rect.contains_rect(bounds));
                assert!(mesh.vertices.iter().all(|vertex| vertex.color.a() == 255));
            }
        }
    }
}

#[test]
fn split_drag_target_retains_seven_point_width_at_each_scale() {
    for scale in [1.0, 1.25, 2.0] {
        for direction in [SplitDirection::SideBySide, SplitDirection::Stacked] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = fixture(directory.path());
            let ctx = egui::Context::default();
            app.actions
                .push(Action::Split(app.focus().unwrap(), direction));
            app.process_actions();
            let split = app.tab().unwrap().root.id();
            let rect = Rect::from_min_size(Pos2::new(8.0, 8.0), Vec2::new(600.0, 400.0));
            render_split_layout(&mut app, &ctx, scale, rect, Vec::new());
            let response = ctx.read_response(Id::new(("split", split))).unwrap();
            let horizontal = direction == SplitDirection::SideBySide;
            assert_eq!(
                if horizontal {
                    response.rect.width()
                } else {
                    response.rect.height()
                },
                7.0
            );
            let initial = response.rect.center();
            let destination = initial
                + if horizontal {
                    Vec2::new(40.0, 0.0)
                } else {
                    Vec2::new(0.0, 40.0)
                };
            for (pos, pressed) in [(initial, true), (destination, true), (destination, false)] {
                let mut events = vec![egui::Event::PointerMoved(pos)];
                if pos == initial || !pressed {
                    events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                render_split_layout(&mut app, &ctx, scale, rect, events);
            }
            let SplitNode::Split { ratio, .. } = app.tab().unwrap().root else {
                unreachable!()
            };
            assert!(
                ratio > 0.5 && ratio < 0.9,
                "divider should still resize its panes"
            );
        }
    }
}

#[test]
fn titlebar_boundary_is_opaque_and_pixel_aligned_while_glass_stays_translucent() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        for sidebar_visible in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = fixture(directory.path());
            app.state.sidebar_visible = sidebar_visible;
            let ctx = egui::Context::default();
            app.load_fonts(&ctx);
            ctx.style_mut(|style| {
                style.spacing.button_padding = Vec2::new(8.0, 5.0);
                style.spacing.item_spacing = Vec2::new(5.0, 6.0);
                style.spacing.menu_margin = egui::Margin::same(8);
            });
            let mut colors = app.settings.colors(true);
            colors.background = colors.background.gamma_multiply(0.5);
            let mut body_top = 0.0;
            let mut output = egui::FullOutput::default();
            for _ in 0..2 {
                let mut input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(801.0, 500.0))),
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(scale);
                output = ctx.run(input, |ctx| {
                    app.toolbar(ctx, &colors);
                    body_top = ctx.available_rect().top();
                    app.sidebar(ctx, &colors);
                    app.terminal_area(ctx, &colors);
                });
            }
            assert_eq!(output.pixels_per_point, scale);
            assert_eq!(
                body_top, 46.0,
                "titlebar controls must retain their fixed height"
            );
            let boundary = (body_top * scale).round();
            let physical_width = (801.0 * scale).round();
            let titlebar_meshes: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|clipped| {
                    if let egui::Shape::Mesh(mesh) = &clipped.shape {
                        let bounds = mesh.calc_bounds();
                        ((bounds.left() * scale).abs() < 0.001
                            && (bounds.width() * scale - physical_width).abs() < 0.001
                            && (bounds.bottom() * scale - boundary).abs() < 0.001)
                            .then_some((clipped, mesh))
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                titlebar_meshes.len(),
                2,
                "full-width glass plus opaque bottom divider"
            );
            let (_, glass) = titlebar_meshes
                .iter()
                .find(|(_, mesh)| mesh.calc_bounds().top() == 0.0)
                .unwrap();
            assert!(
                glass
                    .vertices
                    .iter()
                    .all(|vertex| vertex.color.a() >= 200 && vertex.color.a() < 255),
                "the glass remains translucent; only the boundary becomes opaque"
            );
            let (divider_shape, divider) = titlebar_meshes
                .iter()
                .find(|(_, mesh)| mesh.calc_bounds().top() > 0.0)
                .unwrap();
            assert!(
                divider
                    .vertices
                    .iter()
                    .all(|vertex| vertex.color.a() == 255)
            );
            for vertex in &divider.vertices {
                for value in [vertex.pos.x, vertex.pos.y] {
                    assert!(
                        (value * scale - (value * scale).round()).abs() < 0.001,
                        "divider edges must align with physical pixels at {scale}x"
                    );
                }
            }
            let divider_bounds = divider.calc_bounds();
            assert!((divider_bounds.height() * scale - scale.round().max(1.0)).abs() < 0.001);
            assert_eq!((divider_shape.clip_rect.bottom() * scale).round(), boundary);
            assert!(
                (divider_bounds.right() * scale - physical_width).abs() < 0.001,
                "the opaque row reaches across both sidebar and terminal regions"
            );
            for primitive in ctx.tessellate(vec![(*divider_shape).clone()], scale) {
                if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                    assert!(
                        mesh.vertices.iter().all(|vertex| vertex.color.a() == 255),
                        "tessellation must not add transparent stroke fringes"
                    );
                }
            }
        }
    }
}
