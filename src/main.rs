mod board;
mod custom;
mod online;

use bevy::input::{
    ButtonState,
    keyboard::{Key, KeyboardInput},
};
use bevy::{
    prelude::*,
    window::{MonitorSelection, WindowMode},
};
use board::{Board, Status};
use online::{Online, Operation, Reply};
use rand::prelude::IndexedRandom;

#[derive(Resource)]
struct Game {
    board: Board,
    elapsed: f32,
    flag_mode: bool,
    custom: bool,
    custom_rows: usize,
    custom_columns: usize,
    custom_mines: usize,
    custom_dialog: Option<custom::Settings>,
    settings_path: Option<std::path::PathBuf>,
    settings_error: Option<String>,
    online: Online,
    quit_confirmation: bool,
    help_open: bool,
    zoomed: bool,
    board_scroll: Vec2,
    last_move: Option<(usize, &'static str)>,
    triggered_mine: Option<usize>,
    auto_solver: bool,
    auto_solver_board: bool,
    auto_solver_idle: f32,
    auto_solver_step: f32,
    auto_solver_restart: Option<f32>,
    game_over: Option<GameOver>,
    connection: Option<ConnectionLost>,
    tile_images: Option<TileImages>,
    banner: Handle<Image>,
    chevron: Handle<Image>,
    ruru: Ruru,
    ruru_font: Handle<Font>,
    loss_image: Handle<Image>,
    win_image: Handle<Image>,
}

#[cfg(target_os = "windows")]
#[derive(Resource)]
struct AppIcon(winit::window::Icon);

#[cfg(target_os = "windows")]
fn app_icon() -> Result<winit::window::Icon, Box<dyn std::error::Error>> {
    let image =
        image::load_from_memory(include_bytes!("..\\gfx\\makibishi-icon.png"))?.into_rgba8();
    let (width, height) = image.dimensions();
    Ok(winit::window::Icon::from_rgba(
        image.into_raw(),
        width,
        height,
    )?)
}

#[cfg(target_os = "windows")]
fn set_window_icon(
    mut created: MessageReader<bevy::window::WindowCreated>,
    icon: Res<AppIcon>,
    _main_thread: bevy::ecs::system::NonSendMarker,
) {
    bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        for event in created.read() {
            if let Some(window) = windows.get_window(event.window) {
                window.set_window_icon(Some(icon.0.clone()));
            } else {
                eprintln!("Cannot set app icon: newly created window is unavailable");
            }
        }
    });
}

const FOOTER_HEIGHT: f32 = 80.0;
const MENU_HEIGHT: f32 = 72.0;
const RURU_WIDTH: f32 = 68.0;
const RURU_HEIGHT: f32 = 72.0;
const RURU_TRAVEL_SECONDS: f64 = 3.0;
const RURU_FADE_SECONDS: f64 = 2.0;
const AUTO_SOLVER_IDLE_SECONDS: f32 = 20.0;
const AUTO_SOLVER_RESTART_SECONDS: f32 = 5.0;
const AUTO_SOLVER_BOARD_CHOICES: [(bool, (usize, usize, usize)); 3] = [
    (false, (9, 9, 10)),
    (false, (16, 16, 40)),
    (true, (16, 30, 99)),
];

#[derive(Default)]
struct Ruru {
    clock: f64,
    mine_hits: std::collections::VecDeque<f64>,
    animation_started: Option<f64>,
}

impl Ruru {
    fn record_mine_hit(&mut self) {
        while self
            .mine_hits
            .front()
            .is_some_and(|hit| self.clock - hit > 60.0)
        {
            self.mine_hits.pop_front();
        }
        self.mine_hits.push_back(self.clock);
        if self.mine_hits.len() == 5 {
            self.mine_hits.clear();
            self.animation_started = Some(self.clock);
        }
    }

    fn frame(&self, screen_width: f32) -> Option<(f32, f32)> {
        let elapsed = self.clock - self.animation_started?;
        if elapsed >= RURU_TRAVEL_SECONDS + RURU_FADE_SECONDS {
            return None;
        }
        let progress = (elapsed / RURU_TRAVEL_SECONDS).min(1.0) as f32;
        let center = (screen_width - RURU_WIDTH) / 2.0;
        let left = -RURU_WIDTH + (center + RURU_WIDTH) * progress;
        let alpha = 1.0 - ((elapsed - RURU_TRAVEL_SECONDS) / RURU_FADE_SECONDS).max(0.0) as f32;
        Some((left, alpha))
    }
}

struct TileImages {
    covered: [Handle<Image>; 4],
    flagged: Handle<Image>,
    mine: Handle<Image>,
}

/// A move whose server outcome is unknown because the connection failed.
#[derive(Clone, Copy, PartialEq, Debug)]
struct PendingMove {
    index: usize,
    action: &'static str,
    was_flagged: bool,
}

#[derive(Clone, PartialEq, Debug)]
enum Retry {
    /// Reload the board, then resend `pending` only if the server did not apply it.
    Resync {
        id: String,
        pending: Option<PendingMove>,
    },
    Start {
        mode: String,
    },
    Submit {
        id: String,
    },
}

struct ConnectionLost {
    retry: Retry,
    attempt: u32,
    timer: Timer,
    error: String,
}

/// The popup is shown from the second consecutive failure so short blips stay quiet.
const POPUP_AFTER_ATTEMPTS: u32 = 2;

fn retry_delay(attempt: u32) -> f32 {
    [1.0, 2.0, 4.0, 8.0, 15.0][(attempt as usize).saturating_sub(1).min(4)]
}

const LEADERBOARD_SIZE: usize = 10;

#[derive(Clone, PartialEq, Debug)]
enum Recording {
    Lost,
    Custom,
    Offline,
    Pending,
    Recorded,
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Lookup {
    NotApplicable,
    Waiting,
    Requested,
    Ready,
    Unavailable,
}

#[derive(Clone, Debug)]
struct GameOver {
    won: bool,
    reason: String,
    elapsed_ms: u64,
    mode: Option<&'static str>,
    recording: Recording,
    lookup: Lookup,
}

impl Default for Game {
    fn default() -> Self {
        Self {
            board: Board::new(9, 9, 10),
            elapsed: 0.0,
            flag_mode: false,
            custom: false,
            custom_rows: 10,
            custom_columns: 10,
            custom_mines: 15,
            custom_dialog: None,
            settings_path: None,
            settings_error: None,
            online: Online::default(),
            quit_confirmation: false,
            help_open: false,
            zoomed: false,
            board_scroll: Vec2::ZERO,
            last_move: None,
            triggered_mine: None,
            auto_solver: false,
            auto_solver_board: false,
            auto_solver_idle: 0.0,
            auto_solver_step: 0.0,
            auto_solver_restart: None,
            game_over: None,
            connection: None,
            tile_images: None,
            banner: Handle::default(),
            chevron: Handle::default(),
            ruru: Ruru::default(),
            ruru_font: Handle::default(),
            loss_image: Handle::default(),
            win_image: Handle::default(),
        }
    }
}

#[derive(Component)]
struct Screen;
#[derive(Component)]
struct Tile(usize);
#[derive(Component)]
struct TileFlash {
    timer: Timer,
    original: Color,
}
#[derive(Component)]
struct StatusText;
#[derive(Component)]
struct NetworkStatusSlot;
#[derive(Component)]
struct GameControls;
#[derive(Component)]
struct RuruOverlay;
#[derive(Component)]
struct RuruSprite;
#[derive(Component)]
struct GameMessage;
#[derive(Component)]
struct LossOverlay;
#[derive(Component)]
struct TriggeredMineHighlight;
#[derive(Component)]
struct WinOverlay;
#[derive(Component)]
struct WinningScore;
#[derive(Component)]
struct ScoreLink(i32);
#[derive(Component)]
struct ScorePreview;
#[derive(Component)]
struct ScoreViewport;
#[derive(Component)]
struct LoginDialog;
#[derive(Component)]
struct BoardViewport;
#[derive(Component)]
struct BoardGrid;
#[derive(Component)]
struct BoardChevron;
#[derive(Component, Clone, Copy)]
enum BoardEdge {
    Top,
    Right,
    Bottom,
    Left,
}
#[derive(Component, Clone, Copy)]
enum Action {
    Beginner,
    Intermediate,
    Advanced,
    Custom,
    StartCustom,
    ResetCustom,
    CloseCustom,
    Rows(i32),
    Columns(i32),
    Mines(i32),
    NewGame,
    FlagMode,
    Fullscreen,
    Login,
    Logout,
    Username,
    Password,
    Remember,
    SignIn,
    CancelLogin,
    Ranked,
    Scores,
    BackToGame,
    RefreshBoard,
    SubmitScore,
    ConfirmQuit,
    CancelQuit,
    Help,
    CloseHelp,
    RetryNow,
    StopRetrying,
    Zoom,
}

#[derive(Component)]
struct RetryText;

fn play_intro(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn((
        AudioPlayer::new(asset_server.load("sfx/makibishi_intro.ogg")),
        PlaybackSettings::DESPAWN,
    ));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                #[cfg(not(target_arch = "wasm32"))]
                file_path: env!("CARGO_MANIFEST_DIR").into(),
                #[cfg(target_arch = "wasm32")]
                file_path: "/static/makibishi".into(),
                #[cfg(target_arch = "wasm32")]
                meta_check: bevy::asset::AssetMetaCheck::Never,
                ..default()
            })
            .set(ImagePlugin::default_nearest())
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Makibishi".into(),
                    resolution: (1200, 950).into(),
                    #[cfg(target_arch = "wasm32")]
                    canvas: Some("#makibishi-canvas".into()),
                    #[cfg(target_arch = "wasm32")]
                    fit_canvas_to_parent: true,
                    #[cfg(target_arch = "wasm32")]
                    prevent_default_event_handling: true,
                    ..default()
                }),
                ..default()
            }),
    )
    .insert_resource(ClearColor(Color::srgb(0.07, 0.09, 0.13)))
    .init_resource::<Game>()
    .add_systems(Startup, play_intro)
    .add_systems(Startup, setup)
    .add_systems(
        PostUpdate,
        update_board_edges.after(bevy::ui::UiSystems::Layout),
    )
    .add_systems(
        Update,
        (
            update_ruru,
            login_input,
            interrupt_auto_solver,
            keyboard,
            controls,
            open_score_links,
            dismiss_game_over_on_click,
            tiles,
            run_auto_solver,
            update_flashes,
            poll_online,
            retry_connection,
            update_status,
            scroll_board,
            scroll_scores,
        )
            .chain(),
    );
    #[cfg(target_os = "windows")]
    {
        app.insert_resource(AppIcon(app_icon()?))
            .add_systems(Update, set_window_icon);
        let ui_font = windows_ui_font()?;
        let japanese_font = windows_font("YuGothR.ttc")?;
        let mut fonts = app.world_mut().resource_mut::<Assets<Font>>();
        fonts.insert(bevy::asset::AssetId::default(), ui_font)?;
        let ruru_font = fonts.add(japanese_font);
        app.world_mut().resource_mut::<Game>().ruru_font = ruru_font;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let font = Font::try_from_bytes(WEB_FONT.to_vec())
            .map_err(|error| format!("Cannot load embedded web font: {error}"))?;
        let mut fonts = app.world_mut().resource_mut::<Assets<Font>>();
        fonts.insert(bevy::asset::AssetId::default(), font)?;
        // The default handle now covers Danish and Japanese text, so Ruru can share it.
        app.world_mut().resource_mut::<Game>().ruru_font = Handle::default();
    }
    app.run();
    Ok(())
}

/// Noto Sans JP subset (SIL OFL 1.1, see assets/fonts/OFL.txt) covering Latin-1, punctuation and kana.
#[cfg(any(target_arch = "wasm32", test))]
const WEB_FONT: &[u8] = include_bytes!("../assets/fonts/MakibishiWeb.ttf");

#[cfg(target_os = "windows")]
fn windows_ui_font() -> Result<Font, Box<dyn std::error::Error>> {
    windows_font("segoeui.ttf")
}

#[cfg(target_os = "windows")]
fn windows_font(filename: &str) -> Result<Font, Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(
        std::env::var_os("WINDIR")
            .ok_or("WINDIR is unavailable; cannot locate the Windows UI font")?,
    )
    .join("Fonts")
    .join(filename);
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("Cannot read UI font {}: {error}", path.display()))?;
    Font::try_from_bytes(bytes)
        .map_err(|error| format!("Cannot load UI font {}: {error}", path.display()).into())
}

fn setup(mut commands: Commands, mut game: ResMut<Game>, asset_server: Res<AssetServer>) {
    commands.spawn(Camera2d);
    match custom::path() {
        Ok(path) => {
            match custom::load(&path) {
                Ok(settings) => {
                    game.custom_rows = settings.rows;
                    game.custom_columns = settings.columns;
                    game.custom_mines = settings.mines;
                }
                Err(error) => game.settings_error = Some(error),
            }
            game.settings_path = Some(path);
        }
        Err(error) => game.settings_error = Some(error),
    }
    game.banner = asset_server.load("gfx/Makibishi-Title.png");
    game.chevron = asset_server.load("gfx/chevron.png");
    game.loss_image = asset_server.load("gfx/MonkOuch.png");
    game.win_image = asset_server.load("gfx/MonkAtRest.png");
    spawn_ruru(&mut commands, asset_server.load("gfx/ruru.png"));
    game.tile_images = Some(TileImages {
        covered: std::array::from_fn(|variant| {
            asset_server.load(format!("gfx/tile-{variant}.png"))
        }),
        flagged: asset_server.load("gfx/sashimono.png"),
        mine: asset_server.load("gfx/makibishi-white.png"),
    });
    game.online.dispatch(Operation::Restore);
    build_screen(&mut commands, &game);
}

fn spawn_ruru(commands: &mut Commands, image: Handle<Image>) {
    commands
        .spawn((
            RuruOverlay,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                bottom: px(FOOTER_HEIGHT + MENU_HEIGHT + 4.0),
                width: percent(100),
                height: px(RURU_HEIGHT),
                ..default()
            },
            Visibility::Hidden,
            bevy::ui::FocusPolicy::Pass,
            GlobalZIndex(30),
        ))
        .with_children(|parent| {
            parent.spawn((
                RuruSprite,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(-RURU_WIDTH),
                    width: px(RURU_WIDTH),
                    height: px(RURU_HEIGHT),
                    ..default()
                },
                ImageNode::new(image),
                bevy::ui::FocusPolicy::Pass,
            ));
        });
}

fn update_ruru(
    time: Res<Time<Real>>,
    mut game: ResMut<Game>,
    mut overlays: Query<(&ComputedNode, &mut Visibility), With<RuruOverlay>>,
    mut sprites: Query<(&mut Node, &mut ImageNode), With<RuruSprite>>,
) {
    game.ruru.clock += time.delta_secs_f64();
    for (computed, mut visibility) in &mut overlays {
        let frame = game
            .ruru
            .frame(computed.size().x * computed.inverse_scale_factor);
        *visibility = if frame.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let Some((left, alpha)) = frame {
            for (mut node, mut image) in &mut sprites {
                node.left = px(left);
                image.color.set_alpha(alpha);
            }
        }
    }
}

fn label(text: impl Into<String>, size: f32) -> impl Bundle {
    colored_label(text, size, Color::WHITE)
}

fn tile_variant(row: u32, col: u32) -> usize {
    let mut x = row.wrapping_mul(0x9E3779B9) ^ col.wrapping_mul(0x85EBCA6B);

    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB352D);
    x ^= x >> 15;

    (x % 4) as usize
}

fn colored_label(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
    )
}

fn number_color(count: u8) -> Color {
    match count {
        1 => Color::srgb(0.35, 0.65, 1.0),
        2 => Color::srgb(0.35, 0.85, 0.45),
        3 => Color::srgb(1.0, 0.4, 0.4),
        4 => Color::srgb(0.7, 0.55, 1.0),
        5 => Color::srgb(1.0, 0.6, 0.3),
        6 => Color::srgb(0.3, 0.9, 0.9),
        7 => Color::srgb(1.0, 0.55, 0.8),
        8 => Color::srgb(0.7, 0.75, 0.8),
        _ => Color::WHITE,
    }
}

fn button(parent: &mut ChildSpawnerCommands, text: &str, action: Action) {
    parent
        .spawn((
            Button,
            action,
            Node {
                padding: UiRect::axes(px(12), px(8)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.18, 0.24, 0.34)),
        ))
        .with_children(|parent| {
            parent.spawn(label(text, 16.0));
        });
}

fn footer_button(parent: &mut ChildSpawnerCommands, text: &str, action: Action, color: Color) {
    parent
        .spawn((
            Button,
            action,
            Node {
                padding: UiRect::axes(px(6), px(6)),
                ..default()
            },
        ))
        .with_children(|parent| {
            parent.spawn(colored_label(text, 14.0, color));
        });
}

fn score_preview(parent: &mut ChildSpawnerCommands, board: &online::WinningBoard) {
    parent
        .spawn((
            ScorePreview,
            Node {
                display: Display::Grid,
                grid_template_columns: vec![GridTrack::px(3.0); board.columns],
                grid_template_rows: vec![GridTrack::px(3.0); board.rows],
                flex_shrink: 0.0,
                ..default()
            },
            bevy::ui::FocusPolicy::Pass,
        ))
        .with_children(|preview| {
            for cell in &board.cells {
                let color = if cell.mine {
                    if cell.flagged {
                        Color::srgb(0.9, 0.65, 0.2)
                    } else {
                        Color::srgb(0.7, 0.16, 0.18)
                    }
                } else if cell.count > 0 {
                    number_color(cell.count)
                } else {
                    Color::srgb(0.12, 0.17, 0.23)
                };
                preview.spawn((
                    Node {
                        width: px(3),
                        height: px(3),
                        ..default()
                    },
                    BackgroundColor(color),
                    bevy::ui::FocusPolicy::Pass,
                ));
            }
        });
}

fn score_row(
    parent: &mut ChildSpawnerCommands,
    game: &Game,
    rank: usize,
    score: &online::Score,
    font_size: f32,
    selected: Option<bool>,
) {
    let mut row = parent.spawn(Node {
        width: percent(100),
        padding: UiRect::all(px(6)),
        align_items: AlignItems::Center,
        column_gap: px(8),
        flex_shrink: 0.0,
        ..default()
    });
    if let Some(id) = score.id {
        row.insert((Button, ScoreLink(id)));
    }
    row.with_children(|row| {
        if let Some(selected) = selected {
            let mut marker = row.spawn((
                Node {
                    width: px(9),
                    height: px(14),
                    flex_shrink: 0.0,
                    ..default()
                },
                ImageNode::new(game.chevron.clone()),
                if selected {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
            ));
            if selected {
                marker.insert(WinningScore);
            }
        }
        if let Some(board) = &score.board {
            score_preview(row, board);
        }
        row.spawn(label(
            format!(
                "{}. {}{} - {:.2}s",
                rank + 1,
                score.player_name,
                if score.user_id.is_none() { " *" } else { "" },
                score.elapsed_ms as f64 / 1000.0
            ),
            font_size,
        ));
    });
}

fn ranked_game(game: &Game) -> bool {
    !game.custom && game.online.ranked && game.online.game_id.is_some()
}

fn custom_button(
    parent: &mut ChildSpawnerCommands,
    text: &str,
    action: Action,
    width: f32,
    primary: bool,
) {
    parent
        .spawn((
            Button,
            action,
            Node {
                width: px(width),
                max_width: percent(100),
                padding: UiRect::axes(px(12), px(10)),
                border: UiRect::all(px(1)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(if primary {
                Color::srgb(0.92, 0.92, 0.92)
            } else {
                Color::BLACK
            }),
            BorderColor::all(Color::srgb(0.2, 0.2, 0.2)),
            BorderRadius::all(px(28)),
        ))
        .with_children(|parent| {
            parent.spawn(colored_label(
                text,
                20.0,
                if primary { Color::BLACK } else { Color::WHITE },
            ));
        });
}

fn build_custom_dialog(commands: &mut Commands, game: &Game, settings: custom::Settings) {
    commands
        .spawn((
            Screen,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
            GlobalZIndex(20),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        width: px(560),
                        max_width: percent(95),
                        max_height: percent(95),
                        overflow: Overflow::scroll_y(),
                        flex_direction: FlexDirection::Column,
                        row_gap: px(16),
                        padding: UiRect::all(px(24)),
                        border: UiRect::all(px(1)),
                        ..default()
                    },
                    BackgroundColor(Color::BLACK),
                    BorderColor::all(Color::srgb(0.2, 0.2, 0.2)),
                    BorderRadius::all(px(24)),
                ))
                .with_children(|parent| {
                    parent
                        .spawn(Node {
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|parent| {
                            parent.spawn(label("Custom game", 26.0));
                            footer_button(parent, "X", Action::CloseCustom, Color::WHITE);
                        });
                    for (title, value, minus, plus) in [
                        ("Rows", settings.rows, Action::Rows(-1), Action::Rows(1)),
                        (
                            "Columns",
                            settings.columns,
                            Action::Columns(-1),
                            Action::Columns(1),
                        ),
                        (
                            "Makibishi",
                            settings.mines,
                            Action::Mines(-1),
                            Action::Mines(1),
                        ),
                    ] {
                        parent
                            .spawn(Node {
                                align_items: AlignItems::Center,
                                column_gap: px(16),
                                ..default()
                            })
                            .with_children(|parent| {
                                parent.spawn((
                                    label(title, 22.0),
                                    Node {
                                        flex_grow: 1.0,
                                        ..default()
                                    },
                                ));
                                custom_button(parent, "-", minus, 76.0, false);
                                parent.spawn((
                                    label(value.to_string(), 22.0),
                                    Node {
                                        width: px(48),
                                        ..default()
                                    },
                                ));
                                custom_button(parent, "+", plus, 76.0, false);
                            });
                    }
                    parent.spawn(label(
                        format!(
                            "{} x {} fields - {} makibishi",
                            settings.rows, settings.columns, settings.mines
                        ),
                        18.0,
                    ));
                    if let Some(error) = &game.settings_error {
                        parent.spawn(colored_label(error, 16.0, number_color(3)));
                    }
                    parent
                        .spawn(Node {
                            justify_content: JustifyContent::End,
                            column_gap: px(16),
                            ..default()
                        })
                        .with_children(|parent| {
                            custom_button(parent, "Reset", Action::ResetCustom, 104.0, false);
                            custom_button(parent, "Start", Action::StartCustom, 216.0, true);
                        });
                });
        });
}

fn login_ready(online: &Online) -> bool {
    !online.busy() && !online.username.trim().is_empty() && !online.password.is_empty()
}

fn build_login_dialog(commands: &mut Commands, game: &Game) {
    let online = &game.online;
    let muted = Color::srgb(0.7, 0.7, 0.7);
    commands
        .spawn((
            Screen,
            LoginDialog,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
            GlobalZIndex(20),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        width: px(640),
                        max_width: percent(95),
                        max_height: percent(95),
                        overflow: Overflow::scroll_y(),
                        flex_direction: FlexDirection::Column,
                        row_gap: px(20),
                        padding: UiRect::all(px(24)),
                        border: UiRect::all(px(1)),
                        ..default()
                    },
                    BackgroundColor(Color::BLACK),
                    BorderColor::all(Color::srgb(0.2, 0.2, 0.2)),
                    BorderRadius::all(px(24)),
                ))
                .with_children(|parent| {
                    parent
                        .spawn(Node {
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|parent| {
                            parent.spawn(label("Sign in", 26.0));
                            footer_button(parent, "X", Action::CancelLogin, Color::WHITE);
                        });
                    parent.spawn(colored_label(
                        "Sign in with your wasd.dk account to use online features.",
                        20.0,
                        muted,
                    ));
                    for (title, value, focused, action) in [
                        (
                            "Username",
                            online.username.clone(),
                            !online.password_focus,
                            Action::Username,
                        ),
                        (
                            "Password",
                            "*".repeat(online.password.chars().count()),
                            online.password_focus,
                            Action::Password,
                        ),
                    ] {
                        parent
                            .spawn(Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: px(8),
                                ..default()
                            })
                            .with_children(|parent| {
                                parent.spawn(colored_label(title, 20.0, muted));
                                parent
                                    .spawn((
                                        Button,
                                        action,
                                        Node {
                                            width: percent(100),
                                            min_height: px(56),
                                            padding: UiRect::axes(px(24), px(12)),
                                            border: UiRect::all(px(1)),
                                            align_items: AlignItems::Center,
                                            overflow: Overflow::clip(),
                                            ..default()
                                        },
                                        BackgroundColor(Color::BLACK),
                                        BorderColor::all(if focused {
                                            Color::WHITE
                                        } else {
                                            Color::srgb(0.2, 0.2, 0.2)
                                        }),
                                        BorderRadius::all(px(28)),
                                    ))
                                    .with_children(|parent| {
                                        parent.spawn(colored_label(
                                            if value.is_empty() {
                                                title.to_string()
                                            } else {
                                                value.clone()
                                            },
                                            20.0,
                                            if value.is_empty() {
                                                muted
                                            } else {
                                                Color::WHITE
                                            },
                                        ));
                                    });
                            });
                    }
                    parent
                        .spawn((
                            Button,
                            Action::Remember,
                            Node {
                                align_items: AlignItems::Center,
                                column_gap: px(16),
                                ..default()
                            },
                        ))
                        .with_children(|parent| {
                            parent
                                .spawn((
                                    Node {
                                        width: px(30),
                                        height: px(30),
                                        border: UiRect::all(px(1)),
                                        align_items: AlignItems::Center,
                                        justify_content: JustifyContent::Center,
                                        ..default()
                                    },
                                    BorderColor::all(muted),
                                    BorderRadius::all(px(15)),
                                ))
                                .with_children(|parent| {
                                    if online.remember {
                                        parent.spawn((
                                            Node {
                                                width: px(8),
                                                height: px(16),
                                                border: UiRect {
                                                    right: px(3),
                                                    bottom: px(3),
                                                    ..default()
                                                },
                                                ..default()
                                            },
                                            BorderColor::all(Color::WHITE),
                                            UiTransform::from_rotation(Rot2::degrees(45.0)),
                                        ));
                                    }
                                });
                            parent.spawn(label("Remember login", 22.0));
                        });
                    if !online.message.is_empty() {
                        parent.spawn(colored_label(&online.message, 16.0, muted));
                    }
                    parent
                        .spawn(Node {
                            justify_content: JustifyContent::End,
                            column_gap: px(12),
                            ..default()
                        })
                        .with_children(|parent| {
                            custom_button(parent, "Cancel", Action::CancelLogin, 116.0, false);
                            if login_ready(online) {
                                custom_button(parent, "Sign in", Action::SignIn, 116.0, true);
                            } else {
                                parent
                                    .spawn((
                                        Node {
                                            width: px(116),
                                            padding: UiRect::axes(px(12), px(10)),
                                            align_items: AlignItems::Center,
                                            justify_content: JustifyContent::Center,
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgb(0.04, 0.04, 0.04)),
                                        BorderRadius::all(px(28)),
                                    ))
                                    .with_children(|parent| {
                                        parent.spawn(colored_label(
                                            if online.busy() {
                                                "Signing in..."
                                            } else {
                                                "Sign in"
                                            },
                                            20.0,
                                            muted,
                                        ));
                                    });
                            }
                        });
                });
        });
}

fn build_screen(commands: &mut Commands, game: &Game) {
    if let Some(settings) = game.custom_dialog {
        build_custom_dialog(commands, game, settings);
    }
    if game.online.login_open {
        build_login_dialog(commands, game);
    }
    commands
        .spawn((
            Screen,
            NetworkStatusSlot,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(0),
                left: px(0),
                width: percent(100),
                height: px(FOOTER_HEIGHT),
                min_height: px(FOOTER_HEIGHT),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                flex_wrap: FlexWrap::Wrap,
                column_gap: px(12),
                row_gap: px(4),
                flex_shrink: 0.0,
                padding: UiRect::axes(px(12), px(4)),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            BackgroundColor(Color::BLACK),
            GlobalZIndex(10),
        ))
        .with_children(|parent| {
            parent
                .spawn(Node {
                    column_gap: px(4),
                    align_items: AlignItems::Center,
                    flex_wrap: FlexWrap::Wrap,
                    ..default()
                })
                .with_children(|parent| {
                    let account = game
                        .online
                        .session
                        .as_ref()
                        .map_or("Not signed in".to_string(), |session| {
                            format!("Signed in ({})", session.display_name)
                        });
                    footer_button(
                        parent,
                        &account,
                        if game.online.session.is_some() {
                            Action::Logout
                        } else {
                            Action::Login
                        },
                        Color::WHITE,
                    );
                    parent.spawn(label("\u{00b7}", 14.0));
                    footer_button(
                        parent,
                        if ranked_game(game) {
                            "Online ranked mode"
                        } else {
                            "Offline unranked"
                        },
                        Action::Ranked,
                        if ranked_game(game) {
                            Color::WHITE
                        } else {
                            number_color(3)
                        },
                    );
                    parent.spawn(label("\u{00b7}", 14.0));
                    footer_button(parent, "F2 Leaderboard", Action::Scores, Color::WHITE);
                });
            parent
                .spawn(Node {
                    flex_grow: 1.0,
                    flex_basis: px(140),
                    min_width: px(100),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|parent| {
                    parent.spawn((label("", 14.0), GameMessage));
                    parent.spawn(label(&game.online.message, 14.0));
                    if let Some(error) = &game.settings_error {
                        parent.spawn(colored_label(error, 14.0, number_color(3)));
                    }
                    if game.online.needs_refresh && !game.online.busy() {
                        footer_button(parent, "Refresh board", Action::RefreshBoard, Color::WHITE);
                    }
                    if game.online.game_id.is_some()
                        && game.board.status == Status::Won
                        && !game.online.submitted
                    {
                        footer_button(parent, "Submit score", Action::SubmitScore, Color::WHITE);
                    }
                });
            parent
                .spawn(Node {
                    column_gap: px(4),
                    margin: UiRect::left(Val::Auto),
                    align_items: AlignItems::Center,
                    flex_wrap: FlexWrap::Wrap,
                    ..default()
                })
                .with_children(|parent| {
                    footer_button(
                        parent,
                        if game.flag_mode {
                            "(F) Flag: ON"
                        } else {
                            "(F) Flag"
                        },
                        Action::FlagMode,
                        Color::WHITE,
                    );
                    footer_button(parent, "F1 Help", Action::Help, Color::WHITE);
                    footer_button(
                        parent,
                        if game.zoomed {
                            "F5 Zoom x2"
                        } else {
                            "F5 Zoom x1"
                        },
                        Action::Zoom,
                        Color::WHITE,
                    );
                    footer_button(parent, "F12 Fullscreen", Action::Fullscreen, Color::WHITE);
                });
        });
    commands
        .spawn((
            Screen,
            GameControls,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(FOOTER_HEIGHT),
                width: percent(100),
                height: px(MENU_HEIGHT),
                padding: UiRect::axes(px(16), px(8)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: px(16),
                flex_wrap: FlexWrap::Wrap,
                overflow: Overflow::scroll_y(),
                ..default()
            },
            BackgroundColor(Color::srgb(0.07, 0.09, 0.13)),
            GlobalZIndex(10),
        ))
        .with_children(|parent| {
            parent
                .spawn(Node {
                    column_gap: px(8),
                    flex_wrap: FlexWrap::Wrap,
                    ..default()
                })
                .with_children(|parent| {
                    button(parent, "Beginner", Action::Beginner);
                    button(parent, "Intermediate", Action::Intermediate);
                    button(parent, "Advanced", Action::Advanced);
                    button(parent, "Custom", Action::Custom);
                });
            parent
                .spawn(Node {
                    column_gap: px(12),
                    align_items: AlignItems::Center,
                    flex_wrap: FlexWrap::Wrap,
                    ..default()
                })
                .with_children(|parent| {
                    button(parent, "New game (N)", Action::NewGame);
                    parent.spawn((label("", 16.0), StatusText));
                });
        });
    if game.help_open && !game.quit_confirmation {
        commands.spawn((
            Screen,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100), height: percent(100),
                align_items: AlignItems::Center, justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.9)),
            GlobalZIndex(20),
        )).with_children(|parent| {
            parent.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(12), padding: UiRect::all(px(24)), ..default() })
                .with_children(|parent| {
                    parent.spawn(label("How to play", 28.0));
                    parent.spawn(label("Reveal all safe tiles without stepping on a makibishi.\nNumbers count makibishi in the eight neighboring tiles.\nLeft click: reveal. Right click: flag.\nClick a number or middle / left+right click: chord when flags match.\nOtherwise, middle click flashes uncleared neighbors.\nF: flag mode. N: new game. F2: online leaderboard.\nF5: toggle 2x board zoom. Wheel: scroll; Shift+wheel: sideways.\nF12: fullscreen. Ctrl+Q: quit (confirmation during active games).\nOffline and Custom games are unranked.", 18.0));
                    button(parent, "Close (F1 / Esc)", Action::CloseHelp);
                });
        });
    }
    if let Some(over) = game.game_over.as_ref().filter(|_| !game.quit_confirmation) {
        if !over.won {
            commands
                .spawn((
                    Screen,
                    LossOverlay,
                    Node {
                        position_type: PositionType::Absolute,
                        width: percent(100),
                        height: percent(100),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
                    bevy::ui::FocusPolicy::Block,
                    GlobalZIndex(20),
                ))
                .with_children(|parent| {
                    parent.spawn((
                        Node {
                            height: percent(80),
                            max_width: percent(90),
                            aspect_ratio: Some(891.0 / 881.0),
                            ..default()
                        },
                        ImageNode::new(game.loss_image.clone()),
                    ));
                });
        } else {
            build_win_screen(commands, game, over);
        }
    }
    if let Some(lost) = game
        .connection
        .as_ref()
        .filter(|lost| lost.attempt >= POPUP_AFTER_ATTEMPTS && !game.quit_confirmation)
    {
        commands
            .spawn((
                Screen,
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.07, 0.09, 0.13, 0.96)),
                GlobalZIndex(25),
            ))
            .with_children(|parent| {
                parent
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: px(12),
                            padding: UiRect::all(px(24)),
                            max_width: px(760),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.12, 0.17, 0.23)),
                    ))
                    .with_children(|parent| {
                        parent.spawn(colored_label(
                            "Connection problem",
                            28.0,
                            Color::srgb(1.0, 0.6, 0.3),
                        ));
                        parent.spawn(label(&lost.error, 16.0));
                        let (detail, stop) = match &lost.retry {
                            Retry::Resync { .. } => (
                                if game.board.started {
                                    "The board is hidden until it is back in sync. The ranked clock keeps running on the server."
                                } else {
                                    "The board is hidden until it is back in sync."
                                },
                                "Stop and play offline (Esc)",
                            ),
                            Retry::Start { .. } => (
                                "Could not start a ranked game.",
                                "Stop and play offline (Esc)",
                            ),
                            Retry::Submit { .. } => (
                                "Your winning time has not been saved yet.",
                                "Stop retrying (Esc)",
                            ),
                        };
                        parent.spawn(label(detail, 18.0));
                        parent.spawn((label(retry_text(lost, &game.online), 18.0), RetryText));
                        parent
                            .spawn(Node {
                                column_gap: px(12),
                                ..default()
                            })
                            .with_children(|parent| {
                                button(parent, "Retry now (Enter)", Action::RetryNow);
                                button(parent, stop, Action::StopRetrying);
                            });
                    });
            });
    }
    if game.quit_confirmation {
        commands
            .spawn((
                Screen,
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
                GlobalZIndex(20),
            ))
            .with_children(|parent| {
                parent
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: px(16),
                        padding: UiRect::all(px(24)),
                        ..default()
                    })
                    .with_children(|parent| {
                        parent.spawn(label(
                            "A game is in progress. Quit Makibishi Sweeper?",
                            24.0,
                        ));
                        parent
                            .spawn(Node {
                                column_gap: px(12),
                                ..default()
                            })
                            .with_children(|parent| {
                                button(parent, "Quit (Enter)", Action::ConfirmQuit);
                                button(parent, "Keep playing (Esc)", Action::CancelQuit);
                            });
                    });
            });
    }
    let mut content = commands.spawn((
        Screen,
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: px(12),
            padding: UiRect {
                bottom: px(168),
                ..UiRect::all(px(16))
            },
            overflow: Overflow::scroll_y(),
            ..default()
        },
    ));
    if game.online.show_scores {
        content.insert((ScoreViewport, ScrollPosition::default()));
    }
    content.with_children(|parent| {
        parent.spawn((
            ImageNode::new(game.banner.clone()),
            Node {
                width: percent(100),
                max_width: px(1269),
                aspect_ratio: Some(1269.0 / 249.0),
                flex_shrink: 0.0,
                ..default()
            },
        ));
        let account = if ranked_game(game) {
            game.online
                .session
                .as_ref()
                .map_or("Online".to_string(), |session| {
                    format!("Online as {}", session.display_name)
                })
        } else {
            "Offline".to_string()
        };
        parent.spawn((
            colored_label(
                account,
                16.0,
                if ranked_game(game) {
                    Color::WHITE
                } else {
                    number_color(3)
                },
            ),
            Node {
                align_self: AlignSelf::End,
                ..default()
            },
        ));
        if game.online.show_scores {
            parent.spawn(label(
                "Top 10 server-managed games per difficulty (* = guest account not verified)",
                18.0,
            ));
            if [
                &game.online.scores.easy,
                &game.online.scores.intermediate,
                &game.online.scores.advanced,
            ]
            .into_iter()
            .flatten()
            .any(|score| score.id.is_some())
            {
                parent.spawn(label(
                    "Click a score to view its winning board on the website.",
                    14.0,
                ));
            }
            parent
                .spawn(Node {
                    column_gap: px(30),
                    ..default()
                })
                .with_children(|parent| {
                    for (title, scores) in [
                        ("Beginner", &game.online.scores.easy),
                        ("Intermediate", &game.online.scores.intermediate),
                        ("Advanced", &game.online.scores.advanced),
                    ] {
                        parent
                            .spawn(Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: px(8),
                                ..default()
                            })
                            .with_children(|parent| {
                                parent.spawn(label(title, 22.0));
                                if !game.online.scores_loaded {
                                    parent.spawn(label("Scores have not loaded.", 16.0));
                                } else if scores.is_empty() {
                                    parent.spawn(label("No scores yet.", 16.0));
                                }
                                for (rank, score) in scores.iter().take(10).enumerate() {
                                    score_row(parent, game, rank, score, 16.0, None);
                                }
                            });
                    }
                });
            button(parent, "Back to game", Action::BackToGame);
            return;
        }
        let scale = if game.zoomed { 2.0 } else { 1.0 };
        let tile_size = board_tile_size(game);
        for edge in [
            BoardEdge::Top,
            BoardEdge::Right,
            BoardEdge::Bottom,
            BoardEdge::Left,
        ] {
            let angle = match edge {
                BoardEdge::Top => LinearGradient::TO_BOTTOM,
                BoardEdge::Right => LinearGradient::TO_LEFT,
                BoardEdge::Bottom => LinearGradient::TO_TOP,
                BoardEdge::Left => LinearGradient::TO_RIGHT,
            };
            let rotation = match edge {
                BoardEdge::Top => -90.0,
                BoardEdge::Right => 0.0,
                BoardEdge::Bottom => 90.0,
                BoardEdge::Left => 180.0,
            };
            parent
                .commands()
                .spawn((
                    Screen,
                    edge,
                    Node {
                        position_type: PositionType::Absolute,
                        display: Display::None,
                        ..default()
                    },
                    BackgroundGradient::from(LinearGradient::new(
                        angle,
                        vec![
                            ColorStop::new(Color::srgba(0.0, 0.0, 0.0, 0.5), percent(0)),
                            ColorStop::new(Color::srgba(0.0, 0.0, 0.0, 0.0), percent(100)),
                        ],
                    )),
                    bevy::ui::FocusPolicy::Pass,
                    GlobalZIndex(5),
                ))
                .with_children(|parent| {
                    parent.spawn((
                        BoardChevron,
                        ImageNode::new(game.chevron.clone()),
                        Node {
                            position_type: PositionType::Absolute,
                            width: px(9),
                            height: px(14),
                            ..default()
                        },
                        UiTransform::from_rotation(Rot2::degrees(rotation)),
                        bevy::ui::FocusPolicy::Pass,
                    ));
                });
        }
        parent
            .spawn((
                BoardViewport,
                ScrollPosition(game.board_scroll),
                Node {
                    width: percent(100),
                    flex_grow: 1.0,
                    min_height: px(0),
                    overflow: Overflow::scroll(),
                    ..default()
                },
            ))
            .with_children(|parent| {
                parent
                    .spawn((
                        BoardGrid,
                        Node {
                            display: Display::Grid,
                            grid_template_columns: vec![
                                GridTrack::px(tile_size);
                                game.board.columns
                            ],
                            grid_template_rows: vec![GridTrack::px(tile_size); game.board.rows],
                            column_gap: px(2.0 * scale),
                            row_gap: px(2.0 * scale),
                            margin: UiRect::all(Val::Auto),
                            flex_shrink: 0.0,
                            ..default()
                        },
                    ))
                    .with_children(|parent| {
                        for (index, cell) in game.board.cells.iter().enumerate() {
                            let show_mine =
                                cell.mine && (cell.revealed || game.board.status == Status::Lost);
                            let image = game.tile_images.as_ref().and_then(|images| {
                                if show_mine {
                                    Some(images.mine.clone())
                                } else if cell.flagged {
                                    Some(images.flagged.clone())
                                } else if !cell.revealed {
                                    let row = (index / game.board.columns) as u32;
                                    let col = (index % game.board.columns) as u32;
                                    Some(images.covered[tile_variant(row, col)].clone())
                                } else {
                                    None
                                }
                            });
                            let text = if image.is_some() {
                                String::new()
                            } else if show_mine {
                                "*".into()
                            } else if cell.flagged {
                                "F".into()
                            } else if cell.revealed && cell.count > 0 {
                                cell.count.to_string()
                            } else {
                                String::new()
                            };
                            let color = if show_mine {
                                Color::srgb(0.7, 0.16, 0.18)
                            } else if cell.revealed {
                                Color::srgb(0.12, 0.17, 0.23)
                            } else if cell.flagged {
                                Color::srgb(0.52, 0.36, 0.12)
                            } else {
                                Color::srgb(0.26, 0.34, 0.43)
                            };
                            let mut tile = parent.spawn((
                                Button,
                                Tile(index),
                                Node {
                                    width: px(tile_size),
                                    height: px(tile_size),
                                    align_items: AlignItems::Center,
                                    justify_content: JustifyContent::Center,
                                    ..default()
                                },
                                BackgroundColor(color),
                            ));
                            if let Some(image) = image {
                                tile.insert(ImageNode {
                                    image_mode: NodeImageMode::Stretch,
                                    ..ImageNode::new(image)
                                });
                            }
                            tile.with_children(|parent| {
                                if game.board.status == Status::Lost
                                    && game.triggered_mine == Some(index)
                                {
                                    parent.spawn((
                                        TriggeredMineHighlight,
                                        Node {
                                            position_type: PositionType::Absolute,
                                            width: percent(100),
                                            height: percent(100),
                                            border: UiRect::all(px(3.0 * scale)),
                                            ..default()
                                        },
                                        BorderColor::all(Color::srgb(1.0, 0.85, 0.1)),
                                        bevy::ui::FocusPolicy::Pass,
                                    ));
                                }
                                let text_color = if cell.revealed && !show_mine {
                                    number_color(cell.count)
                                } else {
                                    Color::WHITE
                                };
                                parent.spawn(colored_label(text, 20.0 * scale, text_color));
                            });
                        }
                    });
            });
    });
}

fn build_win_screen(commands: &mut Commands, game: &Game, over: &GameOver) {
    commands
        .spawn((
            Screen,
            WinOverlay,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                align_items: AlignItems::Center,
                column_gap: px(24),
                padding: UiRect::all(px(24)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.9)),
            bevy::ui::FocusPolicy::Block,
            GlobalZIndex(20),
        ))
        .with_children(|parent| {
            parent.spawn((
                Node {
                    width: percent(50),
                    max_width: Val::Vh(90.0 * 921.0 / 760.0),
                    aspect_ratio: Some(921.0 / 760.0),
                    flex_shrink: 0.0,
                    ..default()
                },
                ImageNode::new(game.win_image.clone()),
            ));
            parent
                .spawn((
                    ScoreViewport,
                    ScrollPosition::default(),
                    Node {
                        flex_direction: FlexDirection::Column,
                        flex_grow: 1.0,
                        min_width: px(0),
                        max_height: percent(90),
                        row_gap: px(10),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(|parent| {
                    let mut highlighted = false;
                    if let Some(mode) = over.mode {
                        parent.spawn(label(format!("{} - Top 10", mode_title(mode)), 24.0));
                        match over.lookup {
                            Lookup::Waiting | Lookup::Requested => {
                                parent.spawn(label("Loading leaderboard...", 18.0));
                            }
                            Lookup::Unavailable => {
                                parent.spawn(colored_label(
                                    "Leaderboard unavailable.",
                                    18.0,
                                    number_color(3),
                                ));
                            }
                            Lookup::Ready => {
                                let scores = mode_scores(mode, &game.online);
                                let place = recorded_place(over, &game.online);
                                if scores.iter().any(|score| score.id.is_some()) {
                                    parent.spawn(label(
                                        "Click a score to view its winning board on the website.",
                                        14.0,
                                    ));
                                }
                                if scores.is_empty() {
                                    parent.spawn(label("No scores yet.", 18.0));
                                }
                                for (index, score) in
                                    scores.iter().take(LEADERBOARD_SIZE).enumerate()
                                {
                                    let selected = place == Some(index);
                                    score_row(parent, game, index, score, 18.0, Some(selected));
                                    if selected {
                                        highlighted = true;
                                    }
                                }
                            }
                            Lookup::NotApplicable => {}
                        }
                    }
                    if !highlighted || over.recording != Recording::Recorded {
                        parent.spawn(label(
                            format!("Your time: {:.2}s", over.elapsed_ms as f64 / 1000.0),
                            22.0,
                        ));
                        if over.mode.is_some() {
                            parent.spawn(label(placement_text(over, &game.online), 16.0));
                        }
                    }
                    parent.spawn(label(recording_text(over, &game.online), 16.0));
                    if game
                        .online
                        .message
                        .starts_with("Cannot open winning board:")
                    {
                        parent.spawn(colored_label(&game.online.message, 16.0, number_color(3)));
                    }
                });
        });
}

fn mode_scores<'a>(mode: &str, online: &'a Online) -> &'a [online::Score] {
    match mode {
        "intermediate" => &online.scores.intermediate,
        "advanced" => &online.scores.advanced,
        _ => &online.scores.easy,
    }
}

fn recorded_place(over: &GameOver, online: &Online) -> Option<usize> {
    if over.recording != Recording::Recorded {
        return None;
    }
    let scores = mode_scores(over.mode?, online);
    let player = online.session.as_ref()?.display_name.as_str();
    scores
        .iter()
        .take(LEADERBOARD_SIZE)
        .rposition(|score| score.elapsed_ms == over.elapsed_ms && score.player_name == player)
}

fn board_tile_size(game: &Game) -> f32 {
    (1000.0 / game.board.columns as f32)
        .min(580.0 / game.board.rows as f32)
        .min(34.0)
        * if game.zoomed { 2.0 } else { 1.0 }
}

fn board_edge_rect(edge: BoardEdge, board: Rect, viewport: Rect) -> Option<Rect> {
    let visible = Rect {
        min: board.min.max(viewport.min),
        max: board.max.min(viewport.max),
    };
    if visible.max.cmple(visible.min).any() {
        return None;
    }
    let clipped = match edge {
        BoardEdge::Top => board.min.y < viewport.min.y - 0.5,
        BoardEdge::Right => board.max.x > viewport.max.x + 0.5,
        BoardEdge::Bottom => board.max.y > viewport.max.y + 0.5,
        BoardEdge::Left => board.min.x < viewport.min.x - 0.5,
    };
    if !clipped {
        return None;
    }
    let mut rect = visible;
    match edge {
        BoardEdge::Top => rect.max.y = (rect.min.y + 8.0).min(rect.max.y),
        BoardEdge::Right => rect.min.x = (rect.max.x - 8.0).max(rect.min.x),
        BoardEdge::Bottom => rect.min.y = (rect.max.y - 8.0).max(rect.min.y),
        BoardEdge::Left => rect.max.x = (rect.min.x + 8.0).min(rect.max.x),
    }
    Some(rect)
}

fn update_board_edges(
    viewport: Query<(&ComputedNode, &UiGlobalTransform), With<BoardViewport>>,
    grid: Query<(&ComputedNode, &UiGlobalTransform), With<BoardGrid>>,
    mut edges: Query<(&BoardEdge, &mut Node, &Children), Without<BoardChevron>>,
    mut chevrons: Query<(&mut Node, &mut Visibility), (With<BoardChevron>, Without<BoardEdge>)>,
    windows: Query<&Window>,
) {
    let bounds = |node: &ComputedNode, transform: &UiGlobalTransform| {
        let center = transform.translation * node.inverse_scale_factor;
        Rect::from_center_size(center, node.size() * node.inverse_scale_factor)
    };
    let rects = viewport.single().ok().zip(grid.single().ok());
    let pointer = windows.single().ok().and_then(|window| {
        window.physical_cursor_position().and_then(|position| {
            viewport
                .single()
                .ok()
                .map(|(node, _)| position * node.inverse_scale_factor)
        })
    });
    for (edge, mut node, children) in &mut edges {
        let rect = rects.and_then(|((viewport, viewport_transform), (grid, grid_transform))| {
            board_edge_rect(
                *edge,
                bounds(grid, grid_transform),
                bounds(viewport, viewport_transform),
            )
        });
        if let Some(rect) = rect {
            node.display = Display::Flex;
            node.left = px(rect.min.x);
            node.top = px(rect.min.y);
            node.width = px(rect.width());
            node.height = px(rect.height());
            let center = board_chevron_center(*edge, rect.size());
            for child in children {
                if let Ok((mut chevron, mut visibility)) = chevrons.get_mut(*child) {
                    chevron.left = px(center.x - 4.5);
                    chevron.top = px(center.y - 7.0);
                    *visibility = if pointer_near_chevron(pointer, rect.min + center) {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    };
                }
            }
        } else {
            node.display = Display::None;
        }
    }
}

fn board_chevron_center(edge: BoardEdge, fade_size: Vec2) -> Vec2 {
    match edge {
        BoardEdge::Top => Vec2::new(fade_size.x / 2.0, -4.5),
        BoardEdge::Right => Vec2::new(fade_size.x + 4.5, fade_size.y / 2.0),
        BoardEdge::Bottom => Vec2::new(fade_size.x / 2.0, fade_size.y + 4.5),
        BoardEdge::Left => Vec2::new(-4.5, fade_size.y / 2.0),
    }
}

fn pointer_near_chevron(pointer: Option<Vec2>, center: Vec2) -> bool {
    pointer.is_some_and(|pointer| pointer.distance(center) <= 24.0)
}

fn scroll_board(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    mut game: ResMut<Game>,
    mut viewport: Query<(&ComputedNode, &mut ScrollPosition), With<BoardViewport>>,
) {
    for event in wheel.read() {
        if game.help_open
            || game.custom_dialog.is_some()
            || game.quit_confirmation
            || game.game_over.is_some()
            || game.connection.is_some()
            || game.online.login_open
            || game.online.show_scores
        {
            continue;
        }
        let unit = match event.unit {
            bevy::input::mouse::MouseScrollUnit::Line => 40.0,
            bevy::input::mouse::MouseScrollUnit::Pixel => 1.0,
        };
        let delta = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
            Vec2::new(event.y + event.x, 0.0)
        } else {
            Vec2::new(event.x, event.y)
        };
        for (node, mut position) in &mut viewport {
            let max =
                (node.content_size() - node.size()).max(Vec2::ZERO) * node.inverse_scale_factor;
            position.0 = (position.0 - delta * unit).clamp(Vec2::ZERO, max);
            game.board_scroll = position.0;
        }
    }
}

fn scroll_scores(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    mut viewports: Query<(&ComputedNode, &mut ScrollPosition), With<ScoreViewport>>,
) {
    for event in wheel.read() {
        let unit = match event.unit {
            bevy::input::mouse::MouseScrollUnit::Line => 40.0,
            bevy::input::mouse::MouseScrollUnit::Pixel => 1.0,
        };
        let delta = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
            Vec2::new(event.y + event.x, 0.0)
        } else {
            Vec2::new(event.x, event.y)
        };
        for (node, mut position) in &mut viewports {
            let max =
                (node.content_size() - node.size()).max(Vec2::ZERO) * node.inverse_scale_factor;
            position.0 = (position.0 - delta * unit).clamp(Vec2::ZERO, max);
        }
    }
}

fn rebuild(commands: &mut Commands, screen: &Query<Entity, With<Screen>>, game: &Game) {
    for entity in screen.iter() {
        commands.entity(entity).despawn();
    }
    build_screen(commands, game);
}

fn new_board(game: &mut Game) {
    if game.online.busy() {
        return;
    }
    let (rows, columns, mines) = if game.custom {
        (game.custom_rows, game.custom_columns, game.custom_mines)
    } else {
        (game.board.rows, game.board.columns, game.board.mines)
    };
    game.board = Board::new(rows, columns, mines);
    game.triggered_mine = None;
    game.auto_solver = false;
    game.auto_solver_board = false;
    game.auto_solver_idle = 0.0;
    game.auto_solver_step = 0.0;
    game.auto_solver_restart = None;
    game.last_move = None;
    game.elapsed = 0.0;
    game.board_scroll = Vec2::ZERO;
    game.online.game_id = None;
    game.online.submitted = false;
    game.online.needs_refresh = false;
    game.connection = None;
    if game.online.ranked && !game.custom {
        let mode = match (rows, columns) {
            (16, 16) => "intermediate",
            (16, 30) => "advanced",
            _ => "easy",
        };
        game.online.dispatch(Operation::Start { mode: mode.into() });
    }
}

fn interrupt_auto_solver(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut game: ResMut<Game>,
) {
    if (game.auto_solver || game.auto_solver_restart.is_some())
        && (keys.get_just_pressed().next().is_some() || mouse.get_just_pressed().next().is_some())
    {
        game.auto_solver = false;
        game.auto_solver_restart = None;
        game.auto_solver_idle = 0.0;
    }
}

fn run_auto_solver(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut game: ResMut<Game>,
    screen: Query<Entity, With<Screen>>,
) {
    if let Some(remaining) = game.auto_solver_restart {
        let remaining = remaining - time.delta_secs();
        if remaining <= 0.0 {
            game.auto_solver_restart = None;
            game.game_over = None;
            start_auto_solver_game(&mut game);
            rebuild(&mut commands, &screen, &game);
        } else {
            game.auto_solver_restart = Some(remaining);
        }
        return;
    }
    if game.auto_solver {
        if game.board.status != Status::Playing {
            game.auto_solver = false;
            return;
        }
        game.auto_solver_step += time.delta_secs();
        if game.auto_solver_step < 0.3 {
            return;
        }
        game.auto_solver_step = 0.0;
        if let Some((index, action)) = solver_move(&game.board) {
            apply_move(&mut game, index, action);
            if game.board.status != Status::Playing {
                game.auto_solver = false;
            }
            rebuild(&mut commands, &screen, &game);
        } else {
            game.auto_solver = false;
        }
        return;
    }
    if (game.board.status == Status::Playing && game.board.cells.iter().any(|cell| cell.revealed))
        || game.game_over.is_some()
        || game.custom_dialog.is_some()
        || game.help_open
        || game.quit_confirmation
        || game.connection.is_some()
        || game.online.login_open
        || game.online.show_scores
        || game.online.busy()
        || (game.board.status == Status::Won
            && game.online.game_id.is_some()
            && !game.online.submitted)
    {
        game.auto_solver_idle = 0.0;
        return;
    }
    if keys.get_just_pressed().next().is_some() || mouse.get_just_pressed().next().is_some() {
        game.auto_solver_idle = 0.0;
        return;
    }
    game.auto_solver_idle += time.delta_secs();
    if game.auto_solver_idle < AUTO_SOLVER_IDLE_SECONDS {
        return;
    }

    start_auto_solver_game(&mut game);
    rebuild(&mut commands, &screen, &game);
}

fn start_auto_solver_game(game: &mut Game) {
    let enabled_boards: Vec<_> = AUTO_SOLVER_BOARD_CHOICES
        .iter()
        .filter_map(|(enabled, board)| enabled.then_some(*board))
        .collect();
    let (rows, columns, mines) = *enabled_boards
        .choose(&mut rand::rng())
        .expect("at least one autosolver board must be enabled");
    game.board = Board::new(rows, columns, mines);
    game.custom = false;
    game.auto_solver = true;
    game.auto_solver_board = true;
    game.auto_solver_idle = 0.0;
    game.auto_solver_step = 0.0;
    game.triggered_mine = None;
    game.last_move = None;
    game.elapsed = 0.0;
    game.board_scroll = Vec2::ZERO;
    game.online.game_id = None;
    game.online.needs_refresh = false;
    game.online.submitted = false;
    let first = rand::random_range(0..game.board.cells.len());
    apply_move(game, first, "reveal");
}

fn solver_move(board: &Board) -> Option<(usize, &'static str)> {
    let mut deductions = Vec::new();
    for (index, cell) in board.cells.iter().enumerate() {
        if !cell.revealed || cell.count == 0 {
            continue;
        }
        let neighbors = board.neighbors(index);
        let flags = neighbors
            .iter()
            .filter(|&&neighbor| board.cells[neighbor].flagged)
            .count();
        let covered: Vec<_> = neighbors
            .into_iter()
            .filter(|&neighbor| !board.cells[neighbor].revealed && !board.cells[neighbor].flagged)
            .collect();
        if covered.is_empty() {
            continue;
        }
        if flags == usize::from(cell.count) {
            deductions.extend(covered.into_iter().map(|neighbor| (neighbor, "reveal")));
        } else if flags + covered.len() == usize::from(cell.count) {
            deductions.extend(covered.into_iter().map(|neighbor| (neighbor, "flag")));
        }
    }
    deductions.into_iter().next().or_else(|| {
        let covered: Vec<_> = board
            .cells
            .iter()
            .enumerate()
            .filter_map(|(index, cell)| (!cell.revealed && !cell.flagged).then_some(index))
            .collect();
        (!covered.is_empty()).then(|| (covered[rand::random_range(0..covered.len())], "reveal"))
    })
}

fn start_custom(game: &mut Game) {
    let Some(settings) = game.custom_dialog else {
        return;
    };
    if game.settings_path.is_none() && game.settings_error.is_some() {
        return;
    }
    if let Some(path) = &game.settings_path
        && let Err(error) = custom::save(path, settings)
    {
        game.settings_error = Some(error);
        return;
    }
    game.custom_rows = settings.rows;
    game.custom_columns = settings.columns;
    game.custom_mines = settings.mines;
    game.custom = true;
    game.custom_dialog = None;
    game.settings_error = None;
    new_board(game);
}

fn fullscreen(window: &mut Window) {
    window.mode = if window.mode == WindowMode::Windowed {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
}

fn keyboard(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut game: ResMut<Game>,
    screen: Query<Entity, With<Screen>>,
    mut windows: Query<&mut Window>,
) {
    if game.quit_confirmation {
        if keys.just_pressed(KeyCode::Enter) {
            commands.queue(|world: &mut World| {
                world.write_message(bevy::app::AppExit::Success);
            });
        } else if keys.just_pressed(KeyCode::Escape) {
            game.quit_confirmation = false;
            rebuild(&mut commands, &screen, &game);
        }
        return;
    }
    if keys.just_pressed(KeyCode::KeyQ)
        && (keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight))
    {
        if game.board.status == Status::Playing && game.board.started {
            game.quit_confirmation = true;
            rebuild(&mut commands, &screen, &game);
        } else {
            commands.queue(|world: &mut World| {
                world.write_message(bevy::app::AppExit::Success);
            });
        }
        return;
    }
    if game
        .connection
        .as_ref()
        .is_some_and(|lost| lost.attempt >= POPUP_AFTER_ATTEMPTS)
    {
        if keys.just_pressed(KeyCode::Enter) && !game.online.busy() {
            dispatch_retry(&mut game);
            rebuild(&mut commands, &screen, &game);
        } else if keys.just_pressed(KeyCode::Escape) && !game.online.busy() {
            stop_retrying(&mut game);
            rebuild(&mut commands, &screen, &game);
        }
        return;
    }
    if game.game_over.is_some() {
        if game.auto_solver_restart.is_none() && keys.just_pressed(KeyCode::Escape) {
            game.game_over = None;
            rebuild(&mut commands, &screen, &game);
        }
        return;
    }
    if game.custom_dialog.is_some() {
        if keys.just_pressed(KeyCode::Escape) {
            game.custom_dialog = None;
            rebuild(&mut commands, &screen, &game);
        } else if keys.just_pressed(KeyCode::Enter) && !game.online.busy() {
            start_custom(&mut game);
            rebuild(&mut commands, &screen, &game);
        }
        return;
    }
    if game.online.login_open {
        return;
    }
    if keys.just_pressed(KeyCode::F2) {
        toggle_scores(&mut game.online);
        rebuild(&mut commands, &screen, &game);
        return;
    }
    if keys.just_pressed(KeyCode::F1) || (game.help_open && keys.just_pressed(KeyCode::Escape)) {
        game.help_open = !game.help_open;
        rebuild(&mut commands, &screen, &game);
        return;
    }
    if game.help_open {
        return;
    }
    if keys.just_pressed(KeyCode::F5) {
        game.zoomed = !game.zoomed;
        game.board_scroll = Vec2::ZERO;
        rebuild(&mut commands, &screen, &game);
        return;
    }
    if keys.just_pressed(KeyCode::F12) {
        for mut window in &mut windows {
            fullscreen(&mut window);
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        for mut window in &mut windows {
            window.mode = WindowMode::Windowed;
        }
    }
    if game.online.login_open || game.online.show_scores || game.online.busy() {
        return;
    }
    if keys.just_pressed(KeyCode::KeyN) {
        new_board(&mut game);
    }
    if keys.just_pressed(KeyCode::KeyF) {
        game.flag_mode = !game.flag_mode;
    }
    if keys.just_pressed(KeyCode::KeyN) || keys.just_pressed(KeyCode::KeyF) {
        rebuild(&mut commands, &screen, &game);
    }
}

fn controls(
    mut commands: Commands,
    buttons: Query<(&Interaction, &Action), Changed<Interaction>>,
    mut game: ResMut<Game>,
    screen: Query<Entity, With<Screen>>,
    mut windows: Query<&mut Window>,
) {
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if game.quit_confirmation && !matches!(action, Action::ConfirmQuit | Action::CancelQuit) {
            continue;
        }
        if game.help_open && !matches!(action, Action::Help | Action::CloseHelp) {
            continue;
        }
        if game.game_over.is_some() {
            continue;
        }
        if game.connection.is_some() && !matches!(action, Action::RetryNow | Action::StopRetrying) {
            continue;
        }
        if game.custom_dialog.is_some()
            && !matches!(
                action,
                Action::StartCustom
                    | Action::ResetCustom
                    | Action::CloseCustom
                    | Action::Rows(_)
                    | Action::Columns(_)
                    | Action::Mines(_)
            )
        {
            continue;
        }
        if game.online.login_open
            && !matches!(
                action,
                Action::Username
                    | Action::Password
                    | Action::Remember
                    | Action::SignIn
                    | Action::CancelLogin
            )
        {
            continue;
        }
        if game.online.show_scores && !matches!(action, Action::Scores | Action::BackToGame) {
            continue;
        }
        if game.online.busy()
            && !matches!(
                action,
                Action::Fullscreen
                    | Action::Scores
                    | Action::BackToGame
                    | Action::ConfirmQuit
                    | Action::CancelQuit
                    | Action::Help
                    | Action::CloseHelp
                    | Action::Zoom
                    | Action::FlagMode
            )
        {
            continue;
        }
        match action {
            Action::Help => game.help_open = !game.help_open,
            Action::CloseHelp => game.help_open = false,
            Action::RetryNow => dispatch_retry(&mut game),
            Action::StopRetrying => stop_retrying(&mut game),
            Action::Zoom => {
                game.zoomed = !game.zoomed;
                game.board_scroll = Vec2::ZERO;
            }
            Action::Fullscreen => {
                for mut window in &mut windows {
                    fullscreen(&mut window);
                }
                continue;
            }
            Action::Beginner | Action::Intermediate | Action::Advanced => {
                let (rows, columns, mines) = match action {
                    Action::Intermediate => (16, 16, 40),
                    Action::Advanced => (16, 30, 99),
                    _ => (9, 9, 10),
                };
                game.custom = false;
                game.board = Board::new(rows, columns, mines);
                game.elapsed = 0.0;
                new_board(&mut game);
            }
            Action::Custom => {
                game.custom_dialog = Some(custom::Settings {
                    rows: game.custom_rows,
                    columns: game.custom_columns,
                    mines: game.custom_mines,
                });
            }
            Action::StartCustom => start_custom(&mut game),
            Action::ResetCustom => game.custom_dialog = Some(custom::Settings::default()),
            Action::CloseCustom => game.custom_dialog = None,
            Action::NewGame => new_board(&mut game),
            Action::FlagMode => game.flag_mode = !game.flag_mode,
            Action::Rows(delta) => {
                if let Some(settings) = &mut game.custom_dialog {
                    settings.rows = (settings.rows as i32 + delta).clamp(5, 24) as usize;
                }
            }
            Action::Columns(delta) => {
                if let Some(settings) = &mut game.custom_dialog {
                    settings.columns = (settings.columns as i32 + delta).clamp(5, 30) as usize;
                }
            }
            Action::Mines(delta) => {
                if let Some(settings) = &mut game.custom_dialog {
                    settings.mines = (settings.mines as i32 + delta).max(1) as usize;
                }
            }
            Action::Login => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    game.online.login_open = true;
                }
                #[cfg(target_arch = "wasm32")]
                if let Err(error) = online::web::navigate("/admin/login?next=makibishi") {
                    game.online.message = error;
                }
            }
            Action::Username => game.online.password_focus = false,
            Action::Password => game.online.password_focus = true,
            Action::Remember => game.online.remember = !game.online.remember,
            Action::SignIn => sign_in(&mut game.online),
            Action::CancelLogin => {
                game.online.login_open = false;
                game.online.password.clear();
            }
            Action::Logout => {
                #[cfg(not(target_arch = "wasm32"))]
                game.online.dispatch(Operation::Logout);
                #[cfg(target_arch = "wasm32")]
                {
                    game.online.message = "Use the website navigation to sign out.".into();
                }
            }
            Action::Ranked => {
                if game.online.session.is_none() {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        game.online.login_open = true;
                    }
                    #[cfg(target_arch = "wasm32")]
                    if let Err(error) = online::web::navigate("/admin/login?next=makibishi") {
                        game.online.message = error;
                    }
                } else {
                    game.online.ranked = !game.online.ranked;
                    new_board(&mut game);
                }
            }
            Action::Scores => toggle_scores(&mut game.online),
            Action::BackToGame => game.online.show_scores = false,
            Action::ConfirmQuit => {
                commands.queue(|world: &mut World| {
                    world.write_message(bevy::app::AppExit::Success);
                });
                continue;
            }
            Action::CancelQuit => game.quit_confirmation = false,
            Action::RefreshBoard => {
                if let Some(id) = game.online.game_id.clone() {
                    game.online.dispatch(Operation::Refresh { id });
                }
            }
            Action::SubmitScore => {
                if let Some(id) = game.online.game_id.clone() {
                    game.online.dispatch(Operation::Submit { id });
                }
            }
        }
        if let Some(settings) = &mut game.custom_dialog {
            settings.mines = settings.mines.min(settings.rows * settings.columns - 1);
        }
        rebuild(&mut commands, &screen, &game);
        break;
    }
}

fn open_score_links(
    mut commands: Commands,
    links: Query<(&Interaction, &ScoreLink), Changed<Interaction>>,
    mut game: ResMut<Game>,
    screen: Query<Entity, With<Screen>>,
) {
    for (interaction, link) in &links {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let result = game.online.score_url(link.0).and_then(|url| {
            #[cfg(not(target_arch = "wasm32"))]
            {
                webbrowser::open(&url).map_err(|error| error.to_string())
            }
            #[cfg(target_arch = "wasm32")]
            {
                online::web::open(&url)
            }
        });
        if let Err(error) = result {
            game.online.message = format!("Cannot open winning board: {error}");
            rebuild(&mut commands, &screen, &game);
        }
        break;
    }
}

fn dismiss_game_over_on_click(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    mut game: ResMut<Game>,
    screen: Query<Entity, With<Screen>>,
    links: Query<&Interaction, With<ScoreLink>>,
) {
    if game.game_over.is_some()
        && !links
            .iter()
            .any(|interaction| *interaction != Interaction::None)
        && game.auto_solver_restart.is_none()
        && !game.quit_confirmation
        && mouse.get_just_pressed().next().is_some()
    {
        game.game_over = None;
        rebuild(&mut commands, &screen, &game);
    }
}

fn tiles(
    mut commands: Commands,
    buttons: Query<(
        Entity,
        &Interaction,
        &Tile,
        &BackgroundColor,
        Option<&TileFlash>,
    )>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut game: ResMut<Game>,
    screen: Query<Entity, With<Screen>>,
) {
    if game.board.status != Status::Playing
        || game.custom_dialog.is_some()
        || game.help_open
        || game.quit_confirmation
        || game.game_over.is_some()
        || game.connection.is_some()
        || game.online.login_open
        || game.online.show_scores
        || (game.online.ranked
            && !game.custom
            && !game.auto_solver_board
            && (game.online.busy() || game.online.game_id.is_none() || game.online.needs_refresh))
    {
        return;
    }
    for (_, interaction, tile, _, _) in &buttons {
        let over_tile = *interaction != Interaction::None;
        let chord_click = over_tile
            && (mouse.just_pressed(MouseButton::Middle)
                || (mouse.pressed(MouseButton::Left)
                    && mouse.pressed(MouseButton::Right)
                    && (mouse.just_pressed(MouseButton::Left)
                        || mouse.just_pressed(MouseButton::Right))));
        let right_click =
            mouse.just_pressed(MouseButton::Right) && *interaction == Interaction::Hovered;
        let left_click =
            mouse.just_pressed(MouseButton::Left) && *interaction == Interaction::Pressed;
        if !chord_click && !right_click && !left_click {
            continue;
        }
        if chord_click {
            let neighbors = game.board.neighbors(tile.0);
            let cell = game.board.cells[tile.0];
            let flags = neighbors
                .iter()
                .filter(|&&i| game.board.cells[i].flagged)
                .count();
            if cell.revealed && flags == usize::from(cell.count) {
                apply_move(&mut game, tile.0, "chord");
                rebuild(&mut commands, &screen, &game);
            } else {
                for (entity, _, neighbor, background, flash) in &buttons {
                    let cell = game.board.cells[neighbor.0];
                    if neighbors.contains(&neighbor.0) && !cell.revealed && !cell.flagged {
                        commands.entity(entity).insert(TileFlash {
                            timer: Timer::from_seconds(0.2, TimerMode::Once),
                            original: flash.map_or(background.0, |flash| flash.original),
                        });
                    }
                }
            }
            break;
        } else if right_click || game.flag_mode {
            apply_move(&mut game, tile.0, "flag");
        } else {
            let action = if game.board.cells[tile.0].revealed {
                "chord"
            } else {
                "reveal"
            };
            apply_move(&mut game, tile.0, action);
        }
        rebuild(&mut commands, &screen, &game);
        break;
    }
}

fn apply_move(game: &mut Game, index: usize, action: &'static str) {
    game.last_move = Some((index, action));
    if let Some(id) = game.online.game_id.clone() {
        game.online.needs_refresh = true;
        game.online.dispatch(Operation::Move {
            id,
            action,
            row: index / game.board.columns,
            column: index % game.board.columns,
        });
    } else if action == "flag" {
        game.board.flag(index);
    } else {
        let was_playing = game.board.status == Status::Playing;
        game.board.reveal(index);
        if was_playing && game.board.status != Status::Playing {
            game.triggered_mine = game
                .board
                .cells
                .iter()
                .position(|cell| cell.mine && cell.revealed);
            let elapsed_ms = (game.elapsed * 1000.0).round() as u64;
            finish_game(game, elapsed_ms);
        }
    }
}

fn online_triggered_mine(game: &Game, board: &Board) -> Option<usize> {
    let (index, action) = game.last_move?;
    if board.status != Status::Lost
        || game.board.rows != board.rows
        || game.board.columns != board.columns
        || index >= board.cells.len()
    {
        return None;
    }
    if action == "reveal" {
        return board.cells[index].mine.then_some(index);
    }
    if action == "chord" {
        return game.board.neighbors(index).into_iter().find(|&neighbor| {
            let before = game.board.cells[neighbor];
            !before.revealed && !before.flagged && board.cells[neighbor].mine
        });
    }
    None
}

fn leaderboard_mode(game: &Game) -> Option<&'static str> {
    if game.custom {
        return None;
    }
    match (game.board.rows, game.board.columns, game.board.mines) {
        (9, 9, 10) => Some("easy"),
        (16, 16, 40) => Some("intermediate"),
        (16, 30, 99) => Some("advanced"),
        _ => None,
    }
}

fn mode_title(mode: &str) -> &'static str {
    match mode {
        "intermediate" => "Intermediate",
        "advanced" => "Advanced",
        _ => "Beginner",
    }
}

fn finish_game(game: &mut Game, elapsed_ms: u64) {
    let won = game.board.status == Status::Won;
    if !won {
        game.ruru.record_mine_hit();
    }
    if game.auto_solver && game.auto_solver_board {
        game.auto_solver = false;
        game.auto_solver_restart = Some(AUTO_SOLVER_RESTART_SECONDS);
        game.game_over = None;
        return;
    }
    let reason = if won {
        "Every safe tile was cleared.".to_string()
    } else {
        match game.last_move {
            Some((index, action)) => {
                let row = index / game.board.columns + 1;
                let column = index % game.board.columns + 1;
                if action == "chord" {
                    format!(
                        "Chording row {row}, column {column} revealed a makibishi - a flag was misplaced."
                    )
                } else {
                    format!("You stepped on a makibishi at row {row}, column {column}!")
                }
            }
            None => "You stepped on a makibishi!".to_string(),
        }
    };
    let mode = leaderboard_mode(game);
    let recording = if !won {
        Recording::Lost
    } else if game.auto_solver_board {
        Recording::Offline
    } else if game.online.game_id.is_some() {
        if game.online.submitted {
            Recording::Recorded
        } else {
            Recording::Pending
        }
    } else if mode.is_none() {
        Recording::Custom
    } else {
        Recording::Offline
    };
    game.game_over = Some(GameOver {
        won,
        reason,
        elapsed_ms,
        mode,
        recording,
        lookup: if won && mode.is_some() && !game.auto_solver_board {
            Lookup::Waiting
        } else {
            Lookup::NotApplicable
        },
    });
    request_placement(game);
}

fn request_placement(game: &mut Game) {
    let Some(over) = &game.game_over else {
        return;
    };
    if over.lookup != Lookup::Waiting || over.recording == Recording::Pending || game.online.busy()
    {
        return;
    }
    game.online.dispatch(Operation::Scores);
    let requested = game.online.busy();
    if let Some(over) = &mut game.game_over {
        over.lookup = if requested {
            Lookup::Requested
        } else {
            Lookup::Unavailable
        };
    }
}

fn retry_text(lost: &ConnectionLost, online: &Online) -> String {
    if online.busy() {
        format!("Retrying now (attempt {})...", lost.attempt + 1)
    } else {
        format!(
            "Attempt {} failed. Retrying in {}s.",
            lost.attempt,
            lost.timer.remaining_secs().ceil() as u32
        )
    }
}

/// Schedules a retry for a failed game request. Returns false when it cannot be retried.
fn connection_failed(game: &mut Game, operation: Option<Operation>, error: &str) -> bool {
    let previous = game.connection.take();
    let retry = match operation {
        Some(Operation::Move {
            id,
            action,
            row,
            column,
        }) => {
            let index = row * game.board.columns + column;
            Retry::Resync {
                id,
                pending: Some(PendingMove {
                    index,
                    action,
                    was_flagged: game.board.cells.get(index).is_some_and(|c| c.flagged),
                }),
            }
        }
        Some(Operation::Refresh { id }) => {
            let pending = match previous.as_ref().map(|lost| &lost.retry) {
                Some(Retry::Resync {
                    id: previous_id,
                    pending,
                }) if *previous_id == id => *pending,
                _ => None,
            };
            Retry::Resync { id, pending }
        }
        Some(Operation::Start { mode }) => Retry::Start { mode },
        Some(Operation::Submit { id }) => Retry::Submit { id },
        _ => return false,
    };
    let attempt = previous.map_or(0, |lost| lost.attempt) + 1;
    let delay = retry_delay(attempt);
    game.online.message = format!("Connection problem; retrying in {delay:.0}s.");
    game.connection = Some(ConnectionLost {
        retry,
        attempt,
        timer: Timer::from_seconds(delay, TimerMode::Once),
        error: error.into(),
    });
    true
}

fn dispatch_retry(game: &mut Game) {
    let Some(lost) = &mut game.connection else {
        return;
    };
    lost.timer.reset();
    let operation = match lost.retry.clone() {
        Retry::Resync { id, .. } => Operation::Refresh { id },
        Retry::Start { mode } => Operation::Start { mode },
        Retry::Submit { id } => Operation::Submit { id },
    };
    game.online.dispatch(operation);
}

fn stop_retrying(game: &mut Game) {
    if game.online.busy() {
        return;
    }
    let Some(lost) = game.connection.take() else {
        return;
    };
    if let Retry::Submit { .. } = lost.retry {
        if let Some(over) = &mut game.game_over
            && over.recording == Recording::Pending
        {
            over.recording = Recording::Failed(lost.error);
        }
        game.online.message = "Score not saved. Use Submit score to try again.".into();
    } else {
        game.online.ranked = false;
        new_board(game);
        game.online.message =
            "Stopped retrying; the ranked game was abandoned. Playing offline.".into();
    }
}

fn move_applied(board: &Board, pending: PendingMove) -> bool {
    let cell = board.cells[pending.index];
    match pending.action {
        "flag" => cell.flagged != pending.was_flagged,
        "reveal" => cell.revealed,
        // Chording twice is harmless, so it is always resent.
        _ => false,
    }
}

fn retry_connection(
    mut commands: Commands,
    time: Res<Time>,
    mut game: ResMut<Game>,
    screen: Query<Entity, With<Screen>>,
    mut text: Query<&mut Text, With<RetryText>>,
) {
    if game.online.busy() {
        return;
    }
    let Some(lost) = &mut game.connection else {
        return;
    };
    lost.timer.tick(time.delta());
    if lost.timer.is_finished() {
        dispatch_retry(&mut game);
        rebuild(&mut commands, &screen, &game);
    } else if let Some(lost) = &game.connection {
        for mut text in &mut text {
            **text = retry_text(lost, &game.online);
        }
    }
}

fn recording_text(over: &GameOver, online: &Online) -> String {
    let score = format!("Score of {:.2}s", over.elapsed_ms as f64 / 1000.0);
    match &over.recording {
        Recording::Lost => format!("{score} not recorded: only cleared boards are scored."),
        Recording::Custom => format!("{score} not recorded: custom games are unranked."),
        Recording::Offline if online.session.is_none() => {
            format!("{score} not recorded in offline mode (unranked). Log in to play ranked.")
        }
        Recording::Offline => {
            format!(
                "{score} not recorded in offline mode (unranked). Turn on ranked play to compete."
            )
        }
        Recording::Pending => format!("{score} being recorded online..."),
        Recording::Recorded => format!("{score} recorded online."),
        Recording::Failed(error) => {
            format!("{score} not recorded online: {error} Use Submit score to retry.")
        }
    }
}

fn placement_text(over: &GameOver, online: &Online) -> String {
    let Some(mode) = over.mode else {
        return "No leaderboard placement for this game.".into();
    };
    if !over.won {
        return "No leaderboard placement for a lost game.".into();
    }
    let title = mode_title(mode);
    match over.lookup {
        Lookup::Waiting | Lookup::Requested => return "Checking the leaderboard...".into(),
        Lookup::Unavailable => return "Leaderboard placement unavailable.".into(),
        Lookup::NotApplicable | Lookup::Ready => {}
    }
    let scores = mode_scores(mode, online);
    if over.recording == Recording::Recorded {
        match recorded_place(over, online) {
            Some(index) => format!("Leaderboard: #{} on {title}.", index + 1),
            None => format!("Outside the {title} top {LEADERBOARD_SIZE}."),
        }
    } else {
        let place = scores
            .iter()
            .take(LEADERBOARD_SIZE)
            .filter(|s| s.elapsed_ms <= over.elapsed_ms)
            .count()
            + 1;
        if place <= LEADERBOARD_SIZE {
            format!("Would have placed #{place} on {title}.")
        } else {
            format!("Would not have made the {title} top {LEADERBOARD_SIZE}.")
        }
    }
}

fn sign_in(online: &mut Online) {
    if online.busy() {
        return;
    }
    if online.username.trim().is_empty() || online.password.is_empty() {
        online.message = "Enter your username and password.".into();
        return;
    }

    let operation = Operation::Login {
        username: online.username.clone(),
        password: std::mem::take(&mut online.password),
        remember: online.remember,
    };
    online.dispatch(operation);
}

fn toggle_scores(online: &mut Online) {
    online.show_scores = !online.show_scores;
    if online.show_scores {
        online.login_open = false;
        online.password.clear();
    }
    online.deferred_scores = online.show_scores && online.busy();
    if online.show_scores && !online.busy() {
        online.dispatch(Operation::Scores);
    }
}

fn login_input(
    mut commands: Commands,
    mut events: MessageReader<KeyboardInput>,
    mut game: ResMut<Game>,
    screen: Query<Entity, With<Screen>>,
) {
    let mut changed = false;
    for event in events.read() {
        if !game.online.login_open
            || game.help_open
            || game.quit_confirmation
            || game.game_over.is_some()
            || game.connection.is_some()
            || game.online.busy()
            || event.state != ButtonState::Pressed
        {
            continue;
        }
        match &event.logical_key {
            Key::Tab => game.online.password_focus = !game.online.password_focus,
            Key::Enter => sign_in(&mut game.online),
            Key::Escape => {
                game.online.login_open = false;
                game.online.password.clear();
            }
            Key::Backspace => {
                if game.online.password_focus {
                    game.online.password.pop();
                } else {
                    game.online.username.pop();
                }
            }
            Key::Character(text) => {
                let text = event.text.as_ref().unwrap_or(text);
                let limit = if game.online.password_focus { 72 } else { 128 };
                let field = if game.online.password_focus {
                    &mut game.online.password
                } else {
                    &mut game.online.username
                };
                if field.len() + text.len() <= limit && !text.chars().any(char::is_control) {
                    field.push_str(text);
                }
            }
            _ => continue,
        }
        changed = true;
    }
    if changed {
        rebuild(&mut commands, &screen, &game);
    }
}

fn poll_online(
    mut commands: Commands,
    mut game: ResMut<Game>,
    screen: Query<Entity, With<Screen>>,
) {
    let Some(result) = game.online.poll() else {
        return;
    };
    handle_online_reply(&mut game, result);
    request_placement(&mut game);
    if game.online.deferred_scores && !game.online.busy() {
        game.online.deferred_scores = false;
        if game.online.show_scores {
            game.online.dispatch(Operation::Scores);
        }
    }
    rebuild(&mut commands, &screen, &game);
}

fn handle_online_reply(game: &mut Game, result: Result<Reply, online::ApiError>) {
    match result {
        Err(error) => {
            let operation = game.online.last_game_operation.take();
            if error.retryable
                && !error.unauthorized
                && connection_failed(game, operation, &error.message)
            {
                return;
            }
            game.connection = None;
            if let Some(over) = &mut game.game_over {
                if over.recording == Recording::Pending {
                    over.recording = Recording::Failed(error.message.clone());
                }
                if over.lookup == Lookup::Requested {
                    over.lookup = Lookup::Unavailable;
                }
            }
            game.online.message = error.message;
            if error.unauthorized {
                game.online.session = None;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    game.online.login_open = true;
                }
                #[cfg(target_arch = "wasm32")]
                {
                    game.online
                        .message
                        .push_str(" Sign in again using the website.");
                }
                if game.online.ranked || game.online.game_id.is_some() {
                    game.online.ranked = false;
                    new_board(game);
                    game.online
                        .message
                        .push_str(" Online play stopped; a new offline game is available.");
                }
            }
        }
        Ok(reply) => {
            game.online.message.clear();
            match reply {
                Reply::Restored(session) => {
                    if let Some(session) = session {
                        game.online.session = Some(session);
                        game.online.ranked = true;
                        new_board(game);
                    }
                }
                Reply::LoggedIn(session, warning) => {
                    game.online.session = Some(session);
                    game.online.login_open = false;
                    game.online.ranked = true;
                    new_board(game);
                    if let Some(warning) = warning {
                        game.online.message.push_str(&format!(" {warning}"));
                    }
                }
                Reply::LoggedOut(result) => {
                    game.online.session = None;
                    game.online.ranked = false;
                    game.online.password.clear();
                    new_board(game);
                    game.online.message = match result {
                        Ok(()) => "Signed out; remembered login removed.".into(),
                        Err(error) => format!("Signed out locally, but cleanup failed: {error}"),
                    };
                }
                Reply::Scores(scores) => {
                    game.online.scores = scores;
                    game.online.scores_loaded = true;
                    if let Some(over) = &mut game.game_over
                        && over.lookup == Lookup::Requested
                    {
                        over.lookup = Lookup::Ready;
                    }
                }
                Reply::Board(view) => {
                    let elapsed_ms = view.elapsed_ms;
                    let was_playing = game.board.status == Status::Playing;
                    let recovered = game.connection.take();
                    match view.into_board() {
                        Ok((id, mut board, elapsed)) => {
                            if board.status == Status::Lost
                                && board.rows == game.board.rows
                                && board.columns == game.board.columns
                            {
                                for (index, cell) in board.cells.iter_mut().enumerate() {
                                    if !cell.mine {
                                        cell.revealed &= game.board.cells[index].revealed;
                                    }
                                }
                            }
                            if was_playing {
                                game.triggered_mine = online_triggered_mine(game, &board);
                            } else if board.status == Status::Playing {
                                game.triggered_mine = None;
                            }
                            game.online.game_id = Some(id.clone());
                            game.online.needs_refresh = false;
                            game.board = board;
                            if let Some(elapsed) = elapsed {
                                game.elapsed = elapsed;
                            }
                            if game.board.status == Status::Won && !game.online.submitted {
                                game.online.dispatch(Operation::Submit { id });
                            }
                            if let Some(lost) = recovered {
                                game.online.message = "Reconnected; board back in sync.".into();
                                if let Retry::Resync {
                                    pending: Some(pending),
                                    ..
                                } = lost.retry
                                    && game.board.status == Status::Playing
                                    && !move_applied(&game.board, pending)
                                {
                                    apply_move(game, pending.index, pending.action);
                                    game.online.message =
                                        "Reconnected; resending your last move.".into();
                                }
                            }
                            if was_playing && game.board.status != Status::Playing {
                                let elapsed_ms = elapsed_ms
                                    .unwrap_or_else(|| (game.elapsed * 1000.0).round() as u64);
                                finish_game(game, elapsed_ms);
                            }
                        }
                        Err(error) => game.online.message = error,
                    }
                }
                Reply::Submitted => {
                    game.connection = None;
                    game.online.submitted = true;
                    game.online.message = "Online score saved.".into();
                    if let Some(over) = &mut game.game_over
                        && over.recording == Recording::Pending
                    {
                        over.recording = Recording::Recorded;
                    }
                }
            }
        }
    }
}

fn update_flashes(
    mut commands: Commands,
    time: Res<Time>,
    mut tiles: Query<(
        Entity,
        &mut BackgroundColor,
        Option<&mut ImageNode>,
        &mut TileFlash,
    )>,
) {
    for (entity, mut background, image, mut flash) in &mut tiles {
        flash.timer.tick(time.delta());
        if flash.timer.is_finished() {
            background.0 = flash.original;
            if let Some(mut image) = image {
                image.color = Color::WHITE;
            }
            commands.entity(entity).remove::<TileFlash>();
        } else {
            background.0 = Color::srgb(0.55, 0.65, 0.75);
            if let Some(mut image) = image {
                image.color = Color::srgb(1.0, 0.7, 0.7);
            }
        }
    }
}

fn update_status(
    time: Res<Time>,
    mut game: ResMut<Game>,
    mut text: Query<&mut Text, With<StatusText>>,
    mut messages: Query<(&mut Text, &mut TextFont), (With<GameMessage>, Without<StatusText>)>,
) {
    if game.board.started && game.board.status == Status::Playing {
        game.elapsed += time.delta_secs();
    }
    let flags = game.board.cells.iter().filter(|cell| cell.flagged).count();
    let ruru_visible = game.ruru.frame(0.0).is_some();
    let state = if ruru_visible {
        "\u{30eb}\u{30eb} doesn't judge.."
    } else {
        match game.board.status {
            Status::Playing => "Good luck!",
            Status::Won => "Board cleared!",
            Status::Lost => "You stepped on a makibishi!",
        }
    };
    for mut text in &mut text {
        **text = format!(
            "Makibishi: {} \u{00b7} Time: {:.1}s",
            game.board.mines as isize - flags as isize,
            game.elapsed,
        );
    }
    for (mut text, mut font) in &mut messages {
        **text = state.to_string();
        font.font = if ruru_visible {
            game.ruru_font.clone()
        } else {
            Handle::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_sound_decodes_as_vorbis_audio() {
        use bevy::audio::Decodable;

        let sound = AudioSource {
            bytes: std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("sfx")
                    .join("makibishi_intro.ogg"),
            )
            .unwrap()
            .into(),
        };
        assert!(sound.decoder().any(|sample| sample != 0));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn app_icon_decodes_the_supplied_png() {
        let image = image::load_from_memory(include_bytes!("..\\gfx\\makibishi-icon.png"))
            .unwrap()
            .into_rgba8();
        assert_eq!(image.dimensions(), (32, 32));
        assert!(image.pixels().any(|pixel| pixel.0[3] == 0));
        assert!(image.pixels().any(|pixel| pixel.0[3] > 0));
        app_icon().unwrap();
    }

    #[test]
    fn online_loss_highlights_only_triggered_tile_when_server_reveals_all_mines() {
        for chord in [false, true] {
            let mut game = Game::default();
            game.board.started = true;
            game.board.cells[30].revealed = true;
            game.last_move = Some((
                if chord { 10 } else { 20 },
                if chord { "chord" } else { "reveal" },
            ));
            if chord {
                game.board.cells[10].revealed = true;
                game.board.cells[0].flagged = true;
            }
            let mut json = board_json(false);
            json["status"] = "lost".into();
            for index in [0, 20, 80] {
                json["cells"][index]["mine"] = true.into();
                json["cells"][index]["revealed"] = true.into();
            }
            json["cells"][15]["revealed"] = true.into();
            json["cells"][30]["revealed"] = true.into();
            for dismissed in [false, true] {
                if dismissed {
                    game.game_over = None;
                }
                handle_online_reply(
                    &mut game,
                    Ok(Reply::Board(serde_json::from_value(json.clone()).unwrap())),
                );
                assert_eq!(game.triggered_mine, Some(20));
                assert!(!game.board.cells[15].revealed);
                assert!(game.board.cells[30].revealed);
                let mut app = App::new();
                let mut queue = bevy::ecs::world::CommandQueue::default();
                build_screen(&mut Commands::new(&mut queue, app.world()), &game);
                queue.apply(app.world_mut());
                let mut highlights = app
                    .world_mut()
                    .query_filtered::<&ChildOf, With<TriggeredMineHighlight>>();
                let parent = highlights.single(app.world()).unwrap();
                assert_eq!(app.world().get::<Tile>(parent.parent()).unwrap().0, 20);
            }
            new_board(&mut game);
            assert_eq!(game.triggered_mine, None);
            assert_eq!(game.last_move, None);
        }
    }

    #[test]
    fn autosolver_flags_forced_mines_and_reveals_forced_safe_tiles() {
        let mut board = Board::new(2, 3, 1);
        board.cells[1].revealed = true;
        board.cells[1].count = 1;
        for index in [0, 2, 3, 4] {
            board.cells[index].revealed = true;
        }
        assert_eq!(solver_move(&board), Some((5, "flag")));

        let mut board = Board::new(2, 2, 1);
        board.cells[0].flagged = true;
        board.cells[3].revealed = true;
        board.cells[3].count = 1;
        let (index, action) = solver_move(&board).unwrap();
        assert_eq!(action, "reveal");
        assert!([1, 2].contains(&index));
    }

    #[test]
    fn autosolver_fallback_reveals_a_random_covered_tile() {
        let board = Board::new(5, 5, 3);
        let (index, action) = solver_move(&board).unwrap();
        assert_eq!(action, "reveal");
        assert!(!board.cells[index].revealed);
        assert!(!board.cells[index].flagged);
    }

    #[test]
    fn autosolver_board_finishes_unranked_without_score_lookup() {
        let mut game = Game::default();
        game.board.status = Status::Won;
        game.auto_solver_board = true;
        finish_game(&mut game, 1234);
        let over = game.game_over.as_ref().unwrap();
        assert_eq!(over.recording, Recording::Offline);
        assert_eq!(over.lookup, Lookup::NotApplicable);
        assert!(!game.online.busy());
    }

    #[test]
    fn autosolver_stops_on_key_or_mouse_input_without_replacing_board() {
        for key_input in [true, false] {
            let mut game = Game::default();
            game.auto_solver = true;
            game.auto_solver_board = true;
            game.board.started = true;
            game.board.cells[10].revealed = true;
            let mut app = App::new();
            app.insert_resource(game)
                .init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<ButtonInput<MouseButton>>()
                .add_systems(Update, interrupt_auto_solver);
            if key_input {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyF);
            } else {
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(MouseButton::Left);
            }
            app.update();
            let game = app.world().resource::<Game>();
            assert!(!game.auto_solver);
            assert!(game.auto_solver_board);
            assert!(game.board.started);
            assert!(game.board.cells[10].revealed);
        }
    }

    #[test]
    fn autosolver_starts_a_random_standard_board_after_idle() {
        let mut app = App::new();
        app.insert_resource(Game::default())
            .init_resource::<Time>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, run_auto_solver);
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(20));
        app.update();
        let game = app.world().resource::<Game>();
        assert!(game.auto_solver);
        assert!(game.auto_solver_board);
        assert!(game.board.started);
        assert!(matches!(
            (game.board.rows, game.board.columns, game.board.mines),
            (16, 30, 99)
        ));
        assert!(game.online.game_id.is_none());
    }

    #[test]
    fn autosolver_treats_an_unrevealed_flagged_board_as_no_active_game() {
        let mut game = Game::default();
        game.board.flag(0);
        let mut app = App::new();
        app.insert_resource(game)
            .init_resource::<Time>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, run_auto_solver);
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(20));
        app.update();

        let game = app.world().resource::<Game>();
        assert!(game.auto_solver);
        assert_eq!(
            (game.board.rows, game.board.columns, game.board.mines),
            (16, 30, 99)
        );
    }

    #[test]
    fn autosolver_starts_after_a_completed_game_is_dismissed() {
        let mut game = Game::default();
        game.board.status = Status::Won;
        game.board.started = true;
        game.auto_solver_board = true;
        game.game_over = Some(won_over(1000, Recording::Offline));
        let mut app = App::new();
        app.insert_resource(game)
            .init_resource::<Time>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, run_auto_solver);

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(20));
        app.update();
        assert!(!app.world().resource::<Game>().auto_solver);

        {
            let mut game = app.world_mut().resource_mut::<Game>();
            game.game_over = None;
        }
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(20));
        app.update();

        let game = app.world().resource::<Game>();
        assert!(game.auto_solver);
        assert!(game.game_over.is_none());
    }

    #[test]
    fn autosolver_restarts_after_five_seconds_without_showing_results() {
        for status in [Status::Won, Status::Lost] {
            let mut game = Game::default();
            game.board = Board::new(16, 30, 99);
            game.board.started = true;
            game.board.status = status;
            game.auto_solver = true;
            game.auto_solver_board = true;
            finish_game(&mut game, 1000);
            assert_eq!(game.auto_solver_restart, Some(5.0));
            assert!(game.game_over.is_none());
            assert!(!game.online.busy());

            let mut app = App::new();
            app.insert_resource(game)
                .init_resource::<Time>()
                .init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<ButtonInput<MouseButton>>()
                .add_systems(Update, run_auto_solver);

            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs(4));
            app.update();
            let game = app.world().resource::<Game>();
            assert!(game.game_over.is_none());
            assert!(!game.auto_solver);
            assert_eq!(game.board.status, status);
            assert_eq!(game.auto_solver_restart, Some(1.0));

            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs(1));
            app.update();
            let game = app.world().resource::<Game>();
            assert!(game.game_over.is_none());
            assert!(game.auto_solver);
            assert!(game.auto_solver_board);
            assert_eq!(
                (game.board.rows, game.board.columns, game.board.mines),
                (16, 30, 99)
            );
            assert!(game.board.started);
        }
    }

    #[test]
    fn input_cancels_the_autosolver_restart_pause() {
        for key_input in [true, false] {
            let mut game = Game::default();
            game.board.status = Status::Lost;
            game.auto_solver_board = true;
            game.auto_solver_restart = Some(5.0);
            let mut app = App::new();
            app.insert_resource(game)
                .init_resource::<Time>()
                .init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<ButtonInput<MouseButton>>()
                .add_systems(Update, (interrupt_auto_solver, run_auto_solver).chain());
            if key_input {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::Space);
            } else {
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(MouseButton::Left);
            }
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs(5));
            app.update();
            let game = app.world().resource::<Game>();
            assert!(game.auto_solver_restart.is_none());
            assert!(!game.auto_solver);
            assert_eq!(game.board.status, Status::Lost);
            assert_eq!(game.auto_solver_idle, 0.0);
        }
    }

    #[test]
    fn autosolver_waits_while_a_playing_board_has_revealed_tiles() {
        let mut game = Game::default();
        game.board.started = true;
        game.board.cells[0].revealed = true;
        let mut app = App::new();
        app.insert_resource(game)
            .init_resource::<Time>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, run_auto_solver);
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(20));
        app.update();

        assert!(!app.world().resource::<Game>().auto_solver);
        assert_eq!(app.world().resource::<Game>().auto_solver_idle, 0.0);
        assert!(app.world().resource::<Game>().board.started);
    }

    #[test]
    fn triggered_mine_stays_highlighted_after_dismissal_and_clears_on_new_game() {
        for chord in [false, true] {
            for zoomed in [false, true] {
                let mut game = Game {
                    zoomed,
                    ..default()
                };
                game.board.started = true;
                game.board.cells[0].mine = true;
                game.board.cells[80].mine = true;
                if chord {
                    game.board.cells[1].revealed = true;
                    game.board.cells[1].count = 1;
                    game.board.cells[2].flagged = true;
                }
                apply_move(
                    &mut game,
                    if chord { 1 } else { 0 },
                    if chord { "chord" } else { "reveal" },
                );
                assert_eq!(game.board.status, Status::Lost);
                for dismissed in [false, true] {
                    if dismissed {
                        game.game_over = None;
                    }
                    let mut app = App::new();
                    let mut queue = bevy::ecs::world::CommandQueue::default();
                    build_screen(&mut Commands::new(&mut queue, app.world()), &game);
                    queue.apply(app.world_mut());
                    let mut highlights = app.world_mut().query_filtered::<(
                        &ChildOf,
                        &Node,
                        &BorderColor,
                        &bevy::ui::FocusPolicy,
                    ), With<TriggeredMineHighlight>>(
                    );
                    let (parent, node, border, focus) = highlights.single(app.world()).unwrap();
                    assert_eq!(app.world().get::<Tile>(parent.parent()).unwrap().0, 0);
                    assert_eq!(node.border, UiRect::all(px(if zoomed { 6 } else { 3 })));
                    assert_eq!(*border, BorderColor::all(Color::srgb(1.0, 0.85, 0.1)));
                    assert_eq!(*focus, bevy::ui::FocusPolicy::Pass);
                }
                new_board(&mut game);
                let mut app = App::new();
                let mut queue = bevy::ecs::world::CommandQueue::default();
                build_screen(&mut Commands::new(&mut queue, app.world()), &game);
                queue.apply(app.world_mut());
                assert_eq!(
                    app.world_mut()
                        .query_filtered::<Entity, With<TriggeredMineHighlight>>()
                        .iter(app.world())
                        .count(),
                    0
                );
            }
        }
    }

    fn win_screen_app(mode: Option<&'static str>, recording: Recording, lookup: Lookup) -> App {
        let mut game = Game::default();
        game.board.status = Status::Won;
        game.online.session = Some(online::Session {
            token: "token".into(),
            display_name: "Player".into(),
        });
        game.online.scores.easy = vec![score("Beginner player", 1000)];
        game.online.scores.intermediate =
            vec![score("Intermediate player", 2000), score("Player", 7500)];
        game.online.scores.advanced = vec![score("Advanced player", 3000)];
        game.game_over = Some(GameOver {
            mode,
            lookup,
            ..won_over(7500, recording)
        });
        let mut app = App::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        build_screen(&mut Commands::new(&mut queue, app.world()), &game);
        queue.apply(app.world_mut());
        app.insert_resource(game);
        app
    }

    #[test]
    fn win_screen_has_art_left_and_only_current_mode_scores_right() {
        for (mode, player) in [
            ("easy", "Beginner player"),
            ("intermediate", "Intermediate player"),
            ("advanced", "Advanced player"),
        ] {
            let mut app = win_screen_app(Some(mode), Recording::Recorded, Lookup::Ready);
            let mut overlay = app.world_mut().query_filtered::<
                (&Node, &BackgroundColor, &GlobalZIndex, &Children),
                With<WinOverlay>,
            >();
            let (node, color, z, children) = overlay.single(app.world()).unwrap();
            assert_eq!(node.flex_direction, FlexDirection::Row);
            assert_eq!(color.0.alpha(), 0.9);
            assert!(z.0 < 30);
            assert_eq!(children.len(), 2);
            assert_eq!(
                app.world().get::<ImageNode>(children[0]).unwrap().image,
                app.world().resource::<Game>().win_image
            );
            assert_eq!(
                app.world().get::<Node>(children[0]).unwrap().aspect_ratio,
                Some(921.0 / 760.0)
            );
            let mut text = app.world_mut().query::<&Text>();
            let lines: Vec<_> = text.iter(app.world()).map(|text| text.0.as_str()).collect();
            assert!(lines.iter().any(|text| text.contains(player)));
            assert!(!lines.iter().any(|text| {
                *text == "You won!"
                    || text.starts_with("Every safe tile")
                    || text.starts_with("Click anywhere")
            }));
            for other in ["Beginner player", "Intermediate player", "Advanced player"] {
                if other != player {
                    assert!(!lines.iter().any(|text| text.contains(other)));
                }
            }
        }
    }

    #[test]
    fn win_screen_marks_recorded_score_and_shows_unrecorded_time_below_board() {
        for (recording, highlighted) in [
            (Recording::Recorded, true),
            (Recording::Offline, false),
            (Recording::Failed("Submit failed".into()), false),
        ] {
            let mut app = win_screen_app(Some("intermediate"), recording.clone(), Lookup::Ready);
            let mut selected = app
                .world_mut()
                .query_filtered::<(&ImageNode, &Node, &ChildOf), With<WinningScore>>();
            assert_eq!(selected.iter(app.world()).count(), usize::from(highlighted));
            if highlighted {
                let (image, node, parent) = selected.single(app.world()).unwrap();
                assert_eq!(image.image, app.world().resource::<Game>().chevron);
                assert_eq!(node.width, px(9));
                assert_eq!(node.height, px(14));
                assert_eq!(
                    app.world()
                        .get::<BackgroundColor>(parent.parent())
                        .unwrap()
                        .0
                        .alpha(),
                    0.0
                );
                let children = app.world().get::<Children>(parent.parent()).unwrap();
                assert_eq!(
                    app.world().get::<Text>(children[1]).unwrap().0,
                    "2. Player - 7.50s"
                );
            }
            let mut text = app.world_mut().query::<&Text>();
            let lines: Vec<_> = text.iter(app.world()).map(|text| text.0.as_str()).collect();
            assert_eq!(
                lines.contains(&"Your time: 7.50s"),
                recording != Recording::Recorded
            );
            if recording == Recording::Offline {
                assert!(lines.iter().any(|text| text.contains("unranked")));
            }
            if matches!(recording, Recording::Failed(_)) {
                assert!(lines.iter().any(|text| text.contains("Submit failed")));
            }
        }
        let mut app = win_screen_app(Some("easy"), Recording::Recorded, Lookup::Ready);
        let mut text = app.world_mut().query::<&Text>();
        assert!(
            text.iter(app.world())
                .any(|text| text.0 == "Your time: 7.50s")
        );
    }

    #[test]
    fn recording_status_always_includes_finished_time() {
        let online = Online::default();
        for recording in [
            Recording::Lost,
            Recording::Custom,
            Recording::Offline,
            Recording::Pending,
            Recording::Recorded,
            Recording::Failed("Unavailable".into()),
        ] {
            assert!(
                recording_text(&won_over(7500, recording), &online).starts_with("Score of 7.50s")
            );
        }
        assert!(
            recording_text(&won_over(7500, Recording::Offline), &online)
                .contains("not recorded in offline mode")
        );
    }

    #[test]
    fn win_screen_does_not_show_stale_scores_while_loading_or_after_error() {
        for lookup in [Lookup::Waiting, Lookup::Requested, Lookup::Unavailable] {
            let mut app = win_screen_app(Some("intermediate"), Recording::Pending, lookup);
            let mut text = app.world_mut().query::<&Text>();
            let lines: Vec<_> = text.iter(app.world()).map(|text| text.0.as_str()).collect();
            assert!(
                !lines
                    .iter()
                    .any(|text| text.contains("Intermediate player"))
            );
            assert!(lines.contains(&"Your time: 7.50s"));
            assert!(lines.contains(&if lookup == Lookup::Unavailable {
                "Leaderboard unavailable."
            } else {
                "Loading leaderboard..."
            }));
        }
    }

    #[test]
    fn custom_win_has_time_and_unranked_note_without_scoreboard() {
        let mut app = win_screen_app(None, Recording::Custom, Lookup::NotApplicable);
        let mut text = app.world_mut().query::<&Text>();
        let lines: Vec<_> = text.iter(app.world()).map(|text| text.0.as_str()).collect();
        assert!(lines.contains(&"Your time: 7.50s"));
        assert!(
            lines
                .iter()
                .any(|text| text.contains("custom games are unranked"))
        );
        assert!(
            !lines
                .iter()
                .any(|text| text.contains("Top 10") || text.contains("player"))
        );
    }

    #[test]
    fn win_screen_dismisses_on_click_or_escape_without_starting_another_game() {
        let mut app = win_screen_app(None, Recording::Custom, Lookup::NotApplicable);
        app.init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, (controls, dismiss_game_over_on_click).chain());
        app.world_mut()
            .spawn((Interaction::Pressed, Action::NewGame));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(app.world().resource::<Game>().game_over.is_none());
        assert_eq!(app.world().resource::<Game>().board.status, Status::Won);
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<WinOverlay>>()
                .iter(app.world())
                .count(),
            0
        );
        let mut app = shortcut_app(true, Status::Won);
        app.world_mut().resource_mut::<Game>().game_over = Some(won_over(7500, Recording::Custom));
        press_shortcut(&mut app, &[KeyCode::Escape]);
        assert!(app.world().resource::<Game>().game_over.is_none());
    }

    #[test]
    fn loss_picture_is_centered_over_dimming_but_below_ruru_without_modal_text() {
        let mut game = Game::default();
        game.board.status = Status::Lost;
        finish_game(&mut game, 0);
        let mut app = App::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        spawn_ruru(
            &mut Commands::new(&mut queue, app.world()),
            Handle::default(),
        );
        build_screen(&mut Commands::new(&mut queue, app.world()), &game);
        queue.apply(app.world_mut());
        let mut query = app.world_mut().query_filtered::<
            (&Node, &BackgroundColor, &GlobalZIndex, &Children),
            With<LossOverlay>,
        >();
        let (node, color, z, children) = query.single(app.world()).unwrap();
        assert_eq!(node.align_items, AlignItems::Center);
        assert_eq!(node.justify_content, JustifyContent::Center);
        assert_eq!(color.0.alpha(), 0.8);
        let image = app.world().get::<ImageNode>(children[0]).unwrap();
        assert_eq!(image.image, game.loss_image);
        let loss_z = z.0;
        let mut ruru = app
            .world_mut()
            .query_filtered::<&GlobalZIndex, With<RuruOverlay>>();
        assert!(ruru.single(app.world()).unwrap().0 > loss_z);
        let mut text = app.world_mut().query::<&Text>();
        assert!(!text.iter(app.world()).any(|text| text.0 == "You lost"));
    }

    #[test]
    fn loss_click_dismisses_without_changing_board_or_activating_controls() {
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            let mut app = App::new();
            let mut game = Game::default();
            game.board.status = Status::Lost;
            finish_game(&mut game, 0);
            app.insert_resource(game)
                .init_resource::<ButtonInput<MouseButton>>()
                .add_systems(Update, (controls, dismiss_game_over_on_click).chain());
            app.world_mut()
                .spawn((Interaction::Pressed, Action::Intermediate));
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(button);
            app.update();
            let game = app.world().resource::<Game>();
            assert!(game.game_over.is_none());
            assert_eq!(game.board.status, Status::Lost);
            assert_eq!(game.board.rows, 9);
            assert_eq!(
                app.world_mut()
                    .query_filtered::<Entity, With<LossOverlay>>()
                    .iter(app.world())
                    .count(),
                0
            );
        }
    }

    #[test]
    fn player_facing_text_uses_makibishi() {
        for dialog in 0..4 {
            let mut game = Game::default();
            match dialog {
                0 => game.help_open = true,
                1 => game.custom_dialog = Some(custom::Settings::default()),
                2 => game.quit_confirmation = true,
                _ => {
                    game.board.status = Status::Lost;
                    finish_game(&mut game, 0);
                }
            }
            let mut app = App::new();
            let mut queue = bevy::ecs::world::CommandQueue::default();
            build_screen(&mut Commands::new(&mut queue, app.world()), &game);
            queue.apply(app.world_mut());
            app.insert_resource(game)
                .init_resource::<Time>()
                .add_systems(Update, update_status);
            app.update();
            let mut query = app.world_mut().query::<&Text>();
            for text in query.iter(app.world()) {
                assert!(!text.0.to_lowercase().contains("mine"), "{}", text.0);
            }
            assert!(
                query
                    .iter(app.world())
                    .any(|text| text.0.starts_with("Makibishi: 10"))
            );
        }
    }

    #[test]
    fn ruru_status_message_lasts_through_travel_and_fade_then_restores() {
        let mut app = App::new();
        app.init_resource::<Game>()
            .init_resource::<Time>()
            .add_systems(Update, update_status);
        let ruru_font = Assets::<Font>::default().add(Font {
            data: std::sync::Arc::new(Vec::new()),
        });
        app.world_mut().resource_mut::<Game>().ruru_font = ruru_font.clone();
        let message = app.world_mut().spawn((Text::new(""), GameMessage)).id();
        app.update();
        assert_eq!(app.world().get::<Text>(message).unwrap().0, "Good luck!");
        app.world_mut()
            .resource_mut::<Game>()
            .ruru
            .animation_started = Some(0.0);
        for clock in [0.0, 3.0, 4.999] {
            app.world_mut().resource_mut::<Game>().ruru.clock = clock;
            app.update();
            assert_eq!(
                app.world().get::<Text>(message).unwrap().0,
                "\u{30eb}\u{30eb} doesn't judge.."
            );
            assert_eq!(
                app.world().get::<TextFont>(message).unwrap().font,
                ruru_font
            );
        }
        {
            let mut game = app.world_mut().resource_mut::<Game>();
            game.ruru.clock = 5.0;
            game.board.status = Status::Lost;
        }
        app.update();
        assert_eq!(
            app.world().get::<Text>(message).unwrap().0,
            "You stepped on a makibishi!"
        );
        assert_eq!(
            app.world().get::<TextFont>(message).unwrap().font,
            Handle::default()
        );
    }

    #[test]
    fn ruru_triggers_on_five_hits_in_an_inclusive_rolling_minute() {
        let mut ruru = Ruru::default();
        for clock in [0.0, 15.0, 30.0, 45.0] {
            ruru.clock = clock;
            ruru.record_mine_hit();
            assert!(ruru.animation_started.is_none());
        }
        ruru.clock = 60.0;
        ruru.record_mine_hit();
        assert_eq!(ruru.animation_started, Some(60.0));
        assert!(ruru.mine_hits.is_empty());

        ruru.animation_started = None;
        for clock in [70.0, 85.0, 100.0, 115.0, 130.001] {
            ruru.clock = clock;
            ruru.record_mine_hit();
        }
        assert!(ruru.animation_started.is_none());
        assert_eq!(ruru.mine_hits.len(), 4);
        ruru.clock = 131.0;
        ruru.record_mine_hit();
        assert_eq!(ruru.animation_started, Some(131.0));
    }

    #[test]
    fn ruru_counts_offline_losses_across_new_games_not_flags_or_repeated_clicks() {
        let mut game = Game::default();
        for hit in 0..5 {
            new_board(&mut game);
            game.ruru.clock = f64::from(hit) * 10.0;
            game.board.started = true;
            game.board.cells[0].mine = true;
            apply_move(&mut game, 1, "flag");
            assert_eq!(game.ruru.mine_hits.len(), hit as usize);
            apply_move(&mut game, 0, "reveal");
            apply_move(&mut game, 0, "reveal");
            assert_eq!(game.board.status, Status::Lost);
        }
        assert_eq!(game.ruru.animation_started, Some(40.0));
        new_board(&mut game);
        assert_eq!(game.ruru.animation_started, Some(40.0));
    }

    #[test]
    fn ruru_counts_online_loss_once_even_when_board_is_refreshed() {
        let mut game = Game::default();
        let mut json = board_json(false);
        json["status"] = "lost".into();
        json["cells"][0]["mine"] = true.into();
        json["cells"][0]["revealed"] = true.into();
        for _ in 0..2 {
            let view = serde_json::from_value(json.clone()).unwrap();
            handle_online_reply(&mut game, Ok(Reply::Board(view)));
        }
        assert_eq!(game.ruru.mine_hits.len(), 1);
    }

    #[test]
    fn ruru_moves_to_screen_center_then_fades_without_moving() {
        let mut ruru = Ruru {
            animation_started: Some(0.0),
            ..default()
        };
        assert_eq!(ruru.frame(1200.0), Some((-68.0, 1.0)));
        ruru.clock = 1.5;
        assert_eq!(ruru.frame(1200.0), Some((249.0, 1.0)));
        ruru.clock = 3.0;
        assert_eq!(ruru.frame(1200.0), Some((566.0, 1.0)));
        ruru.clock = 4.0;
        assert_eq!(ruru.frame(1200.0), Some((566.0, 0.5)));
        assert_eq!(ruru.frame(800.0), Some((366.0, 0.5)));
        ruru.clock = 5.0;
        assert_eq!(ruru.frame(1200.0), None);
    }

    #[test]
    fn ruru_overlay_survives_screen_rebuild_and_updates_sprite_opacity() {
        let mut app = App::new();
        let mut game = Game::default();
        game.ruru.animation_started = Some(0.0);
        game.ruru.clock = 4.0;
        app.insert_resource(game)
            .init_resource::<Time<Real>>()
            .add_systems(Update, update_ruru);
        let mut queue = bevy::ecs::world::CommandQueue::default();
        spawn_ruru(
            &mut Commands::new(&mut queue, app.world()),
            Handle::default(),
        );
        build_screen(
            &mut Commands::new(&mut queue, app.world()),
            app.world().resource::<Game>(),
        );
        queue.apply(app.world_mut());
        let screens: Vec<_> = app
            .world_mut()
            .query_filtered::<Entity, With<Screen>>()
            .iter(app.world())
            .collect();
        for entity in screens {
            app.world_mut().despawn(entity);
        }
        app.update();
        let mut overlays = app
            .world_mut()
            .query_filtered::<(&Node, &Visibility, &bevy::ui::FocusPolicy), With<RuruOverlay>>();
        let (node, visibility, focus) = overlays.single(app.world()).unwrap();
        assert_eq!(node.bottom, px(156));
        assert_eq!(*visibility, Visibility::Inherited);
        assert_eq!(*focus, bevy::ui::FocusPolicy::Pass);
        let mut sprites = app
            .world_mut()
            .query_filtered::<(&Node, &ImageNode), With<RuruSprite>>();
        let (node, image) = sprites.single(app.world()).unwrap();
        assert_eq!(node.width, px(68));
        assert_eq!(node.height, px(72));
        assert_eq!(image.color.alpha(), 0.5);
        app.world_mut().resource_mut::<Game>().ruru.clock = 5.0;
        app.update();
        assert_eq!(*overlays.single(app.world()).unwrap().1, Visibility::Hidden);
    }

    #[test]
    fn chevrons_are_centered_just_outside_each_fade() {
        assert_eq!(
            board_chevron_center(BoardEdge::Top, Vec2::new(100.0, 8.0)),
            Vec2::new(50.0, -4.5)
        );
        assert_eq!(
            board_chevron_center(BoardEdge::Bottom, Vec2::new(100.0, 8.0)),
            Vec2::new(50.0, 12.5)
        );
        assert_eq!(
            board_chevron_center(BoardEdge::Left, Vec2::new(8.0, 100.0)),
            Vec2::new(-4.5, 50.0)
        );
        assert_eq!(
            board_chevron_center(BoardEdge::Right, Vec2::new(8.0, 100.0)),
            Vec2::new(12.5, 50.0)
        );
        let center = Vec2::new(50.0, 50.0);
        assert!(pointer_near_chevron(Some(center), center));
        assert!(pointer_near_chevron(Some(center + Vec2::X * 24.0), center));
        assert!(!pointer_near_chevron(Some(center + Vec2::X * 24.1), center));
        assert!(!pointer_near_chevron(None, center));
    }

    #[test]
    fn chevrons_rotate_east_facing_art_to_each_cardinal_direction() {
        let mut app = App::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        build_screen(
            &mut Commands::new(&mut queue, app.world()),
            &Game::default(),
        );
        queue.apply(app.world_mut());
        let mut query = app.world_mut().query::<(&BoardEdge, &Children)>();
        assert_eq!(query.iter(app.world()).count(), 4);
        for (edge, children) in query.iter(app.world()) {
            let chevron = children[0];
            assert!(app.world().get::<BoardChevron>(chevron).is_some());
            assert!(app.world().get::<ImageNode>(chevron).is_some());
            let rotation = app.world().get::<UiTransform>(chevron).unwrap().rotation;
            let expected = match edge {
                BoardEdge::Top => -90.0,
                BoardEdge::Right => 0.0,
                BoardEdge::Bottom => 90.0,
                BoardEdge::Left => 180.0,
            };
            assert_eq!(rotation, Rot2::degrees(expected));
        }
    }

    #[test]
    fn clipped_board_edges_are_eight_pixels_inside_the_visible_board() {
        let viewport = Rect::new(0.0, 0.0, 100.0, 100.0);
        let board = Rect::new(-20.0, -30.0, 120.0, 130.0);
        for (edge, expected) in [
            (BoardEdge::Top, Rect::new(0.0, 0.0, 100.0, 8.0)),
            (BoardEdge::Right, Rect::new(92.0, 0.0, 100.0, 100.0)),
            (BoardEdge::Bottom, Rect::new(0.0, 92.0, 100.0, 100.0)),
            (BoardEdge::Left, Rect::new(0.0, 0.0, 8.0, 100.0)),
        ] {
            assert_eq!(board_edge_rect(edge, board, viewport), Some(expected));
            assert_eq!(board_edge_rect(edge, viewport, viewport), None);
        }
        let board = Rect::new(20.0, -30.0, 80.0, 100.0);
        assert_eq!(
            board_edge_rect(BoardEdge::Top, board, viewport),
            Some(Rect::new(20.0, 0.0, 80.0, 8.0))
        );
        assert_eq!(board_edge_rect(BoardEdge::Bottom, board, viewport), None);
        assert_eq!(board_edge_rect(BoardEdge::Left, board, viewport), None);
        assert_eq!(board_edge_rect(BoardEdge::Right, board, viewport), None);
        assert_eq!(
            board_edge_rect(
                BoardEdge::Top,
                Rect::new(0.0, -50.0, 100.0, -10.0),
                viewport
            ),
            None
        );
    }

    #[test]
    fn board_edge_overlays_fade_from_half_black_to_transparent_and_pass_input() {
        let mut app = App::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        build_screen(
            &mut Commands::new(&mut queue, app.world()),
            &Game::default(),
        );
        queue.apply(app.world_mut());
        let mut query = app
            .world_mut()
            .query_filtered::<(&BackgroundGradient, &bevy::ui::FocusPolicy), With<BoardEdge>>();
        assert_eq!(query.iter(app.world()).count(), 4);
        for (gradient, focus) in query.iter(app.world()) {
            assert_eq!(*focus, bevy::ui::FocusPolicy::Pass);
            let Gradient::Linear(gradient) = &gradient.0[0] else {
                panic!("Expected a linear edge fade");
            };
            assert_eq!(gradient.stops[0].color, Color::srgba(0.0, 0.0, 0.0, 0.5));
            assert_eq!(gradient.stops[1].color, Color::srgba(0.0, 0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn custom_settings_only_apply_on_new_game() {
        let mut game = Game {
            custom: true,
            custom_rows: 24,
            custom_columns: 30,
            custom_mines: 100,
            elapsed: 12.0,
            ..default()
        };
        assert_eq!(game.board.rows, 9);
        new_board(&mut game);
        assert_eq!(
            (game.board.rows, game.board.columns, game.board.mines),
            (24, 30, 100)
        );
        assert_eq!(game.elapsed, 0.0);
    }

    fn click_control(app: &mut App, action: Action) {
        let entity = app.world_mut().spawn((Interaction::Pressed, action)).id();
        app.update();
        app.world_mut().despawn(entity);
    }

    #[test]
    fn custom_modal_edits_are_isolated_until_start_and_reset_restores_defaults() {
        let mut app = App::new();
        app.init_resource::<Game>().add_systems(Update, controls);
        {
            let mut game = app.world_mut().resource_mut::<Game>();
            game.custom_rows = 16;
            game.custom_columns = 20;
            game.custom_mines = 40;
            game.board.started = true;
            game.elapsed = 12.0;
        }
        click_control(&mut app, Action::Custom);
        assert_eq!(
            app.world().resource::<Game>().custom_dialog,
            Some(custom::Settings {
                rows: 16,
                columns: 20,
                mines: 40
            })
        );
        click_control(&mut app, Action::Rows(1));
        click_control(&mut app, Action::Advanced);
        {
            let game = app.world().resource::<Game>();
            assert_eq!(game.board.rows, 9);
            assert_eq!(game.custom_rows, 16);
            assert_eq!(game.elapsed, 12.0);
            assert!(game.board.started);
        }
        click_control(&mut app, Action::CloseCustom);
        click_control(&mut app, Action::Custom);
        assert_eq!(
            app.world().resource::<Game>().custom_dialog.unwrap().rows,
            16
        );
        click_control(&mut app, Action::ResetCustom);
        assert_eq!(
            app.world().resource::<Game>().custom_dialog,
            Some(custom::Settings::default())
        );
        click_control(&mut app, Action::StartCustom);
        let game = app.world().resource::<Game>();
        assert_eq!(
            (game.board.rows, game.board.columns, game.board.mines),
            (10, 10, 15)
        );
        assert!(game.custom);
        assert!(game.custom_dialog.is_none());
        assert_eq!(game.elapsed, 0.0);
    }

    #[test]
    fn custom_modal_clamps_dimensions_and_mines() {
        let mut app = App::new();
        app.init_resource::<Game>().add_systems(Update, controls);
        click_control(&mut app, Action::Custom);
        click_control(&mut app, Action::Rows(-100));
        click_control(&mut app, Action::Columns(-100));
        click_control(&mut app, Action::Mines(1000));
        assert_eq!(
            app.world().resource::<Game>().custom_dialog,
            Some(custom::Settings {
                rows: 5,
                columns: 5,
                mines: 24
            })
        );
        click_control(&mut app, Action::Mines(-1000));
        assert_eq!(
            app.world().resource::<Game>().custom_dialog.unwrap().mines,
            1
        );
    }

    #[test]
    fn custom_modal_blocks_game_shortcuts_and_escape_cancels() {
        let mut app = shortcut_app(true, Status::Playing);
        app.world_mut().resource_mut::<Game>().custom_dialog = Some(custom::Settings::default());
        press_shortcut(
            &mut app,
            &[KeyCode::KeyN, KeyCode::KeyF, KeyCode::F2, KeyCode::F5],
        );
        let game = app.world().resource::<Game>();
        assert!(game.board.started);
        assert!(!game.flag_mode);
        assert!(!game.zoomed);
        assert!(!game.online.show_scores);
        press_shortcut(&mut app, &[KeyCode::Escape]);
        assert!(app.world().resource::<Game>().custom_dialog.is_none());
        assert!(app.world().resource::<Game>().board.started);
        app.world_mut().resource_mut::<Game>().custom_dialog = Some(custom::Settings::default());
        press_shortcut(&mut app, &[KeyCode::Enter]);
        assert!(app.world().resource::<Game>().custom);
        assert_eq!(app.world().resource::<Game>().board.rows, 10);
    }

    #[test]
    fn custom_start_persists_settings_and_reports_write_errors() {
        let directory = std::env::temp_dir().join(format!(
            "minesweeper-start-custom-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let path = directory.join("custom.json");
        let settings = custom::Settings {
            rows: 12,
            columns: 14,
            mines: 25,
        };
        let mut game = Game {
            custom_dialog: Some(settings),
            settings_path: Some(path.clone()),
            ..default()
        };
        start_custom(&mut game);
        assert_eq!(custom::load(&path).unwrap(), settings);
        assert!(game.custom_dialog.is_none());
        game.custom_dialog = Some(custom::Settings::default());
        game.settings_path = Some(path.join("cannot-write.json"));
        start_custom(&mut game);
        assert!(game.settings_error.is_some());
        assert!(game.custom_dialog.is_some());
        assert_eq!(game.board.rows, 12);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn custom_modal_blocks_tile_interactions() {
        let mut app = chord_app();
        app.world_mut().resource_mut::<Game>().custom_dialog = Some(custom::Settings::default());
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.update();
        assert_eq!(app.world().resource::<Game>().board.status, Status::Playing);
        assert_eq!(
            app.world_mut()
                .query::<&TileFlash>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn f12_toggles_fullscreen() {
        let mut app = shortcut_app(false, Status::Playing);
        let window = app.world_mut().spawn(Window::default()).id();
        press_shortcut(&mut app, &[KeyCode::F12]);
        assert_eq!(
            app.world().get::<Window>(window).unwrap().mode,
            WindowMode::BorderlessFullscreen(MonitorSelection::Current)
        );
        press_shortcut(&mut app, &[KeyCode::Escape]);
        assert_eq!(
            app.world().get::<Window>(window).unwrap().mode,
            WindowMode::Windowed
        );
    }

    #[test]
    fn fullscreen_toggles_back_to_windowed() {
        let mut window = Window::default();
        fullscreen(&mut window);
        assert_eq!(
            window.mode,
            WindowMode::BorderlessFullscreen(MonitorSelection::Current)
        );
        fullscreen(&mut window);
        assert_eq!(window.mode, WindowMode::Windowed);
    }

    #[test]
    fn difficulty_button_changes_board() {
        let mut app = App::new();
        app.init_resource::<Game>().add_systems(Update, controls);
        app.world_mut()
            .spawn((Interaction::Pressed, Action::Advanced));
        app.update();
        let game = app.world().resource::<Game>();
        assert_eq!(
            (game.board.rows, game.board.columns, game.board.mines),
            (16, 30, 99)
        );
        assert!(!game.custom);
    }

    #[test]
    fn right_click_flags_without_revealing() {
        let mut app = App::new();
        app.init_resource::<Game>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, tiles);
        app.world_mut()
            .spawn((Interaction::Hovered, Tile(0), BackgroundColor(Color::BLACK)));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.update();
        let game = app.world().resource::<Game>();
        assert!(game.board.cells[0].flagged);
        assert!(!game.board.started);
    }

    fn chord_app() -> App {
        let mut app = App::new();
        let mut game = Game {
            board: Board::new(2, 2, 1),
            ..default()
        };
        game.board.started = true;
        game.board.cells[0].mine = true;
        for index in 1..4 {
            game.board.cells[index].count = 1;
        }
        game.board.cells[3].revealed = true;
        app.insert_resource(game)
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Time>()
            .add_systems(Update, (tiles, update_flashes).chain());
        for index in 0..4 {
            app.world_mut().spawn((
                if index == 3 {
                    Interaction::Hovered
                } else {
                    Interaction::None
                },
                Tile(index),
                BackgroundColor(Color::BLACK),
            ));
        }
        app
    }

    #[test]
    fn middle_click_chords_even_in_flag_mode() {
        let mut app = chord_app();
        {
            let mut game = app.world_mut().resource_mut::<Game>();
            game.flag_mode = true;
            game.board.flag(0);
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.update();
        assert_eq!(app.world().resource::<Game>().board.status, Status::Won);
    }

    #[test]
    fn simultaneous_left_right_chords() {
        let mut app = chord_app();
        app.world_mut().resource_mut::<Game>().board.flag(0);
        {
            let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            mouse.press(MouseButton::Left);
            mouse.press(MouseButton::Right);
        }
        app.world_mut()
            .query::<(&Tile, &mut Interaction)>()
            .iter_mut(app.world_mut())
            .for_each(|(tile, mut interaction)| {
                if tile.0 == 3 {
                    *interaction = Interaction::Pressed;
                }
            });
        app.update();
        assert_eq!(app.world().resource::<Game>().board.status, Status::Won);
    }

    #[test]
    fn failed_chord_flashes_only_uncleared_unflagged_neighbors_then_restores() {
        let mut app = chord_app();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&TileFlash>()
                .iter(app.world())
                .count(),
            3
        );
        assert_eq!(app.world().resource::<Game>().board.status, Status::Playing);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(201));
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&TileFlash>()
                .iter(app.world())
                .count(),
            0
        );
        for background in app
            .world_mut()
            .query::<&BackgroundColor>()
            .iter(app.world())
        {
            assert_eq!(background.0, Color::BLACK);
        }
    }

    #[test]
    fn login_and_restored_login_start_fresh_ranked_games() {
        for restored in [false, true] {
            let (online, worker) = online::tests::starting_online();
            let mut game = Game {
                online,
                ..default()
            };
            game.board.flag(0);
            let session = online::Session {
                token: "b".repeat(64),
                display_name: "Player".into(),
            };
            let reply = if restored {
                Reply::Restored(Some(session))
            } else {
                Reply::LoggedIn(session, None)
            };
            handle_online_reply(&mut game, Ok(reply));
            assert!(game.online.session.is_some());
            assert!(game.online.ranked);
            assert!(game.online.busy());
            assert!(!game.board.cells[0].flagged);
            let requests = worker.join().unwrap();
            assert!(requests[0].starts_with("POST /api/player/minesweeper/games "));
        }
    }

    #[test]
    fn expired_online_session_explicitly_stops_ranked_play() {
        let mut game = Game::default();
        game.online.ranked = true;
        game.online.game_id = Some("00000000-0000-0000-0000-000000000000".into());
        handle_online_reply(
            &mut game,
            Err(online::ApiError {
                message: "Session expired.".into(),
                unauthorized: true,
                retryable: false,
            }),
        );
        assert!(!game.online.ranked);
        assert!(game.online.game_id.is_none());
        assert!(game.online.login_open);
        assert!(game.online.message.contains("Online play stopped"));
        assert!(!game.board.started);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_ui_font_contains_danish_glyphs() {
        let font = windows_ui_font().unwrap();
        let face = ttf_parser::Face::parse(&font.data, 0).unwrap();
        for character in [
            '\u{00e6}', '\u{00f8}', '\u{00e5}', '\u{00c6}', '\u{00d8}', '\u{00c5}',
        ] {
            assert!(
                face.glyph_index(character).is_some(),
                "Missing glyph for {character}"
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn ruru_font_contains_japanese_and_english_message_glyphs() {
        let font = windows_font("YuGothR.ttc").unwrap();
        let face = ttf_parser::Face::parse(&font.data, 0).unwrap();
        for character in "\u{30eb}\u{30eb} doesn't judge..".chars() {
            assert!(
                face.glyph_index(character).is_some(),
                "Missing glyph for {character}"
            );
        }
    }

    #[test]
    fn web_font_contains_danish_japanese_and_separator_glyphs() {
        let face = ttf_parser::Face::parse(WEB_FONT, 0).unwrap();
        for character in "\u{00e6}\u{00f8}\u{00e5}\u{00c6}\u{00d8}\u{00c5}\u{00b7}\u{2014}\u{30eb}\u{30eb} doesn't judge.. Makibishi: 0 Time: 1.0s"
            .chars()
        {
            assert!(
                face.glyph_index(character).is_some(),
                "Missing glyph for {character}"
            );
        }
    }

    #[test]
    fn request_status_keeps_a_fixed_slot_and_hides_refresh_during_move() {
        let (online, worker) = online::tests::starting_online();
        let mut game = Game {
            online,
            ..default()
        };
        game.online.session = Some(online::Session {
            token: "b".repeat(64),
            display_name: "Player".into(),
        });
        let mut app = App::new();
        for busy in [false, true] {
            if busy {
                game.online.needs_refresh = true;
                game.online.dispatch(Operation::Start {
                    mode: "easy".into(),
                });
            }
            let mut queue = bevy::ecs::world::CommandQueue::default();
            build_screen(&mut Commands::new(&mut queue, app.world()), &game);
            queue.apply(app.world_mut());
            let slot = app
                .world_mut()
                .query_filtered::<&Node, With<NetworkStatusSlot>>()
                .single(app.world())
                .unwrap();
            assert_eq!(slot.height, px(80));
            assert_eq!(slot.min_height, px(80));
            assert_eq!(slot.flex_shrink, 0.0);
            assert_eq!(slot.position_type, PositionType::Absolute);
            assert_eq!(slot.bottom, px(0));
            assert!(
                !app.world_mut()
                    .query::<&Action>()
                    .iter(app.world())
                    .any(|action| matches!(action, Action::RefreshBoard))
            );
            let screens: Vec<_> = app
                .world_mut()
                .query_filtered::<Entity, With<Screen>>()
                .iter(app.world())
                .collect();
            for screen in screens {
                app.world_mut().despawn(screen);
            }
        }
        worker.join().unwrap();
    }

    #[test]
    fn uncertain_online_move_blocks_more_moves_until_refresh() {
        let mut app = chord_app();
        {
            let mut game = app.world_mut().resource_mut::<Game>();
            game.online.ranked = true;
            game.online.game_id = Some("00000000-0000-0000-0000-000000000000".into());
            game.online.needs_refresh = true;
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&TileFlash>()
                .iter(app.world())
                .count(),
            0
        );
        assert_eq!(app.world().resource::<Game>().board.status, Status::Playing);
    }

    #[test]
    fn invalid_login_preserves_the_current_offline_board() {
        let mut game = Game::default();
        game.board.flag(0);
        handle_online_reply(
            &mut game,
            Err(online::ApiError {
                message: "Invalid username or password.".into(),
                unauthorized: true,
                retryable: false,
            }),
        );
        assert!(game.board.cells[0].flagged);
        assert!(game.online.message.contains("Invalid username"));
    }

    #[test]
    fn keyboard_shortcuts_do_not_trigger_while_editing_login() {
        let mut app = App::new();
        let mut game = Game::default();
        game.online.login_open = true;
        game.board.flag(0);
        app.insert_resource(game)
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, keyboard);
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::KeyN);
            keys.press(KeyCode::KeyF);
        }
        app.update();
        let game = app.world().resource::<Game>();
        assert!(game.board.cells[0].flagged);
        assert!(!game.flag_mode);
    }

    #[test]
    fn login_fields_accept_text_tab_and_backspace_and_mask_the_password() {
        let mut app = App::new();
        let mut game = Game::default();
        game.online.login_open = true;
        app.insert_resource(game)
            .add_message::<KeyboardInput>()
            .add_systems(Update, login_input);
        for logical_key in [
            Key::Character("nf".into()),
            Key::Tab,
            Key::Character("test-password".into()),
            Key::Backspace,
            Key::Shift,
        ] {
            app.world_mut().write_message(KeyboardInput {
                key_code: KeyCode::KeyN,
                logical_key,
                state: ButtonState::Pressed,
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
        }
        app.update();
        let game = app.world().resource::<Game>();
        assert_eq!(game.online.username, "nf");
        assert_eq!(game.online.password, "test-passwor");
        let labels: Vec<_> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|t| t.0.clone())
            .collect();
        assert!(labels.iter().any(|text| text == "************"));
        assert!(labels.iter().all(|text| !text.contains("test-passwor")));
    }

    #[test]
    fn login_is_a_black_modal_over_the_board_with_disabled_empty_submit() {
        let mut game = Game::default();
        game.online.login_open = true;
        game.online.message = "Invalid username or password.".into();
        let mut app = App::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        build_screen(&mut Commands::new(&mut queue, app.world()), &game);
        queue.apply(app.world_mut());
        let dialog = app
            .world_mut()
            .query_filtered::<Entity, With<LoginDialog>>()
            .single(app.world())
            .unwrap();
        assert_eq!(app.world().get::<GlobalZIndex>(dialog).unwrap().0, 20);
        let card = app.world().get::<Children>(dialog).unwrap()[0];
        assert_eq!(
            app.world().get::<BackgroundColor>(card).unwrap().0,
            Color::BLACK
        );
        assert!(app.world().get::<BorderRadius>(card).is_some());
        assert_eq!(
            app.world_mut().query::<&Tile>().iter(app.world()).count(),
            81
        );
        assert!(
            !app.world_mut()
                .query::<&Action>()
                .iter(app.world())
                .any(|action| matches!(action, Action::SignIn))
        );
        assert!(
            app.world_mut()
                .query::<&Text>()
                .iter(app.world())
                .any(|text| text.0 == "Invalid username or password.")
        );
    }

    #[test]
    fn login_fields_focus_remember_and_cancel_preserve_board() {
        let mut game = Game::default();
        game.online.login_open = true;
        game.online.username = "Player".into();
        game.online.password = "secret".into();
        game.board.flag(0);
        let mut app = App::new();
        app.insert_resource(game).add_systems(Update, controls);
        click_control(&mut app, Action::Password);
        assert!(app.world().resource::<Game>().online.password_focus);
        click_control(&mut app, Action::Username);
        assert!(!app.world().resource::<Game>().online.password_focus);
        click_control(&mut app, Action::Remember);
        assert!(app.world().resource::<Game>().online.remember);
        click_control(&mut app, Action::Advanced);
        assert_eq!(app.world().resource::<Game>().board.rows, 9);
        click_control(&mut app, Action::CancelLogin);
        let game = app.world().resource::<Game>();
        assert!(!game.online.login_open);
        assert!(game.online.password.is_empty());
        assert!(game.board.cells[0].flagged);
    }

    #[test]
    fn filled_login_enables_submit_and_masks_password() {
        let mut game = Game::default();
        game.online.login_open = true;
        game.online.username = "Player".into();
        game.online.password = "secret".into();
        game.online.password_focus = true;
        let mut app = App::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        build_screen(&mut Commands::new(&mut queue, app.world()), &game);
        queue.apply(app.world_mut());
        assert!(
            app.world_mut()
                .query::<&Action>()
                .iter(app.world())
                .any(|action| matches!(action, Action::SignIn))
        );
        assert!(
            app.world_mut()
                .query::<&Text>()
                .iter(app.world())
                .any(|text| text.0 == "******")
        );
        assert!(
            !app.world_mut()
                .query::<&Text>()
                .iter(app.world())
                .any(|text| text.0.contains("secret"))
        );
        let mut query = app.world_mut().query::<(&Action, &BorderColor)>();
        let (_, border) = query
            .iter(app.world())
            .find(|(action, _)| matches!(action, Action::Password))
            .unwrap();
        assert_eq!(*border, BorderColor::all(Color::WHITE));
    }

    #[test]
    fn remembered_login_checkmark_uses_geometry_instead_of_a_font() {
        for remember in [false, true] {
            let mut game = Game::default();
            game.online.login_open = true;
            game.online.remember = remember;
            let mut app = App::new();
            let mut queue = bevy::ecs::world::CommandQueue::default();
            build_screen(&mut Commands::new(&mut queue, app.world()), &game);
            queue.apply(app.world_mut());
            let mut query = app
                .world_mut()
                .query::<(&Node, &UiTransform, &BorderColor)>();
            let marks = query
                .iter(app.world())
                .filter(|(node, transform, color)| {
                    node.width == px(8)
                        && node.height == px(16)
                        && node.border.right == px(3)
                        && node.border.bottom == px(3)
                        && transform.rotation == Rot2::degrees(45.0)
                        && **color == BorderColor::all(Color::WHITE)
                })
                .count();
            assert_eq!(marks, usize::from(remember));
        }
    }

    #[test]
    fn login_blocks_function_shortcuts_and_escape_clears_password() {
        let mut app = shortcut_app(true, Status::Playing);
        {
            let mut game = app.world_mut().resource_mut::<Game>();
            game.online.login_open = true;
            game.online.password = "secret".into();
        }
        press_shortcut(&mut app, &[KeyCode::F1, KeyCode::F2, KeyCode::F5]);
        let game = app.world().resource::<Game>();
        assert!(!game.help_open);
        assert!(!game.online.show_scores);
        assert!(!game.zoomed);
        app.add_message::<KeyboardInput>()
            .add_systems(Update, login_input);
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::Escape,
            logical_key: Key::Escape,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        app.update();
        assert!(!app.world().resource::<Game>().online.login_open);
        assert!(app.world().resource::<Game>().online.password.is_empty());
    }

    #[test]
    fn numbers_have_distinct_colors() {
        for count in 1..=8 {
            assert_ne!(number_color(count), Color::WHITE);
            for other in count + 1..=8 {
                assert_ne!(number_color(count), number_color(other));
            }
        }
    }

    #[test]
    fn banner_replaces_title_and_preserves_its_aspect_ratio() {
        let mut assets = Assets::<Image>::default();
        let game = Game {
            banner: assets.add(Image::default()),
            ..default()
        };
        let mut app = App::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        build_screen(&mut Commands::new(&mut queue, app.world()), &game);
        queue.apply(app.world_mut());

        let mut query = app.world_mut().query::<(&ImageNode, &Node)>();
        let (image, node) = query
            .iter(app.world())
            .find(|(image, _)| image.image == game.banner)
            .unwrap();
        assert_eq!(image.image, game.banner);
        assert_eq!(node.width, percent(100));
        assert_eq!(node.max_width, px(1269));
        assert_eq!(node.aspect_ratio, Some(1269.0 / 249.0));
        assert_eq!(node.flex_shrink, 0.0);
        assert!(
            !app.world_mut()
                .query::<&Text>()
                .iter(app.world())
                .any(|text| text.0 == "Minesweeper")
        );
    }

    #[test]
    fn tile_images_cover_hidden_flagged_and_mine_states_only() {
        let mut game = Game {
            board: Board::new(1, 4, 1),
            ..default()
        };
        game.board.status = Status::Lost;
        game.board.cells[1].flagged = true;
        game.board.cells[2].mine = true;
        game.board.cells[2].revealed = true;
        game.board.cells[3].revealed = true;
        game.board.cells[3].count = 1;
        game.tile_images = Some(TileImages {
            covered: std::array::from_fn(|_| Handle::default()),
            flagged: Handle::default(),
            mine: Handle::default(),
        });

        let mut app = App::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        build_screen(&mut Commands::new(&mut queue, app.world()), &game);
        queue.apply(app.world_mut());

        let mut query = app.world_mut().query::<(&Tile, Option<&ImageNode>)>();
        let images: Vec<_> = query
            .iter(app.world())
            .filter_map(|(tile, image)| {
                image.map(|image| {
                    assert_eq!(image.image_mode, NodeImageMode::Stretch);
                    tile.0
                })
            })
            .collect();
        assert_eq!(images, [0, 1, 2]);
    }

    #[test]
    fn tile_variants_match_the_fixed_coordinate_hash() {
        for (row, col, expected) in [
            (0, 0, 0),
            (0, 1, 0),
            (0, 2, 3),
            (0, 3, 2),
            (1, 0, 2),
            (1, 1, 2),
            (2, 3, 3),
            (u32::MAX, u32::MAX, 2),
        ] {
            assert_eq!(tile_variant(row, col), expected);
        }
        let variants: std::collections::HashSet<_> = (0..9)
            .flat_map(|row| (0..9).map(move |col| tile_variant(row, col)))
            .collect();
        assert_eq!(variants.len(), 4);
    }

    #[test]
    fn covered_tiles_use_coordinate_variants_at_both_zoom_levels() {
        let mut assets = Assets::<Image>::default();
        let mut game = Game::default();
        game.tile_images = Some(TileImages {
            covered: std::array::from_fn(|_| assets.add(Image::default())),
            flagged: assets.add(Image::default()),
            mine: assets.add(Image::default()),
        });
        for zoomed in [false, true] {
            game.zoomed = zoomed;
            let mut app = App::new();
            let mut queue = bevy::ecs::world::CommandQueue::default();
            build_screen(&mut Commands::new(&mut queue, app.world()), &game);
            queue.apply(app.world_mut());

            let mut query = app.world_mut().query::<(&Tile, &ImageNode)>();
            let mut count = 0;
            for (tile, image) in query.iter(app.world()) {
                let row = (tile.0 / game.board.columns) as u32;
                let col = (tile.0 % game.board.columns) as u32;
                assert_eq!(
                    image.image,
                    game.tile_images.as_ref().unwrap().covered[tile_variant(row, col)]
                );
                count += 1;
            }
            assert_eq!(count, game.board.cells.len());
        }
    }

    #[test]
    fn f5_doubles_tiles_numbers_and_gaps_without_changing_game() {
        let mut app = shortcut_app(true, Status::Playing);
        let mut widths = Vec::new();
        let mut fonts = Vec::new();
        let mut gaps = Vec::new();
        for zoom in [false, true] {
            if zoom {
                press_shortcut(&mut app, &[KeyCode::F5]);
            } else {
                let mut queue = bevy::ecs::world::CommandQueue::default();
                build_screen(
                    &mut Commands::new(&mut queue, app.world()),
                    app.world().resource::<Game>(),
                );
                queue.apply(app.world_mut());
            }
            let mut query = app.world_mut().query::<(&Tile, &Node, &Children)>();
            let (_, node, children) = query.iter(app.world()).next().unwrap();
            widths.push(node.width);
            fonts.push(app.world().get::<TextFont>(children[0]).unwrap().font_size);
            gaps.push(
                app.world_mut()
                    .query::<&Node>()
                    .iter(app.world())
                    .find(|node| node.display == Display::Grid)
                    .unwrap()
                    .column_gap,
            );
        }
        let (Val::Px(normal), Val::Px(zoom)) = (widths[0], widths[1]) else {
            panic!("Expected pixel sizes");
        };
        assert_eq!(zoom, normal * 2.0);
        assert_eq!(fonts[1], fonts[0] * 2.0);
        assert_eq!(gaps, [px(2), px(4)]);
        assert!(app.world().resource::<Game>().board.started);
        press_shortcut(&mut app, &[KeyCode::F5]);
        assert!(!app.world().resource::<Game>().zoomed);
    }

    #[test]
    fn f1_help_is_modal_and_escape_closes_it() {
        let mut app = shortcut_app(true, Status::Playing);
        press_shortcut(&mut app, &[KeyCode::F1]);
        assert!(app.world().resource::<Game>().help_open);
        let labels: Vec<_> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|t| t.0.clone())
            .collect();
        assert!(labels.iter().any(|t| t == "How to play"));
        press_shortcut(&mut app, &[KeyCode::KeyN]);
        assert!(app.world().resource::<Game>().board.started);
        press_shortcut(&mut app, &[KeyCode::Escape]);
        assert!(!app.world().resource::<Game>().help_open);
    }

    #[test]
    fn stats_and_difficulty_controls_are_above_the_black_status_bar() {
        let mut app = App::new();
        let game = Game::default();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        build_screen(&mut Commands::new(&mut queue, app.world()), &game);
        queue.apply(app.world_mut());
        let status = app
            .world_mut()
            .query_filtered::<Entity, With<NetworkStatusSlot>>()
            .single(app.world())
            .unwrap();
        let stats = app
            .world_mut()
            .query_filtered::<Entity, With<StatusText>>()
            .single(app.world())
            .unwrap();
        let row = app.world().get::<ChildOf>(stats).unwrap().parent();
        let controls = app
            .world_mut()
            .query_filtered::<Entity, With<GameControls>>()
            .single(app.world())
            .unwrap();
        assert_eq!(app.world().get::<ChildOf>(row).unwrap().parent(), controls);
        assert_eq!(
            app.world().get::<BackgroundColor>(status).unwrap().0,
            Color::BLACK
        );
        for (entity, action) in app
            .world_mut()
            .query::<(Entity, &Action)>()
            .iter(app.world())
        {
            let parent = app.world().get::<ChildOf>(entity).unwrap().parent();
            if matches!(
                action,
                Action::FlagMode | Action::Fullscreen | Action::Scores
            ) {
                assert_eq!(app.world().get::<ChildOf>(parent).unwrap().parent(), status);
            } else if matches!(
                action,
                Action::Beginner
                    | Action::Intermediate
                    | Action::Advanced
                    | Action::Custom
                    | Action::NewGame
            ) {
                assert_eq!(
                    app.world().get::<ChildOf>(parent).unwrap().parent(),
                    controls
                );
            }
        }
    }

    #[test]
    fn header_and_footer_distinguish_ranked_and_offline_games() {
        for (signed_in, custom) in [(false, false), (true, false), (true, true)] {
            let mut game = Game {
                custom,
                ..default()
            };
            if signed_in {
                game.online.session = Some(online::Session {
                    token: "b".repeat(64),
                    display_name: "Player".into(),
                });
                game.online.ranked = true;
                game.online.game_id = Some("00000000-0000-0000-0000-000000000000".into());
            }
            let ranked = signed_in && !custom;
            let mut app = App::new();
            let mut queue = bevy::ecs::world::CommandQueue::default();
            build_screen(&mut Commands::new(&mut queue, app.world()), &game);
            queue.apply(app.world_mut());
            let mut query = app
                .world_mut()
                .query::<(&Text, &TextColor, Option<&Node>)>();
            let (header, color, node) = query
                .iter(app.world())
                .find(|(text, _, _)| {
                    text.0
                        == if ranked {
                            "Online as Player"
                        } else {
                            "Offline"
                        }
                })
                .unwrap();
            assert!(!header.0.is_empty());
            assert_eq!(
                color.0,
                if ranked {
                    Color::WHITE
                } else {
                    number_color(3)
                }
            );
            assert_eq!(node.unwrap().align_self, AlignSelf::End);
            let (_, color, _) = query
                .iter(app.world())
                .find(|(text, _, _)| {
                    text.0
                        == if ranked {
                            "Online ranked mode"
                        } else {
                            "Offline unranked"
                        }
                })
                .unwrap();
            assert_eq!(
                color.0,
                if ranked {
                    Color::WHITE
                } else {
                    number_color(3)
                }
            );
        }
    }

    #[test]
    fn centered_board_has_fixed_rows_and_equal_gaps_at_both_zoom_levels() {
        for zoomed in [false, true] {
            let game = Game {
                zoomed,
                ..default()
            };
            let mut app = App::new();
            let mut queue = bevy::ecs::world::CommandQueue::default();
            build_screen(&mut Commands::new(&mut queue, app.world()), &game);
            queue.apply(app.world_mut());
            let mut query = app.world_mut().query::<&Node>();
            let grid = query
                .iter(app.world())
                .find(|node| node.display == Display::Grid)
                .unwrap();
            assert_eq!(
                grid.grid_template_rows,
                vec![GridTrack::px(board_tile_size(&game)); game.board.rows]
            );
            assert_eq!(grid.row_gap, grid.column_gap);
            assert_eq!(grid.row_gap, px(if zoomed { 4 } else { 2 }));
            assert_eq!(grid.margin, UiRect::all(Val::Auto));
        }
    }

    fn shortcut_app(started: bool, status: Status) -> App {
        let mut app = App::new();
        let mut game = Game::default();
        game.board.started = started;
        game.board.status = status;
        app.insert_resource(game)
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<bevy::app::AppExit>()
            .add_systems(Update, keyboard);
        app
    }

    fn press_shortcut(app: &mut App, keys: &[KeyCode]) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.reset_all();
        for &key in keys {
            input.press(key);
        }
        app.update();
    }

    #[test]
    fn ctrl_q_prompts_for_active_game_and_escape_cancels() {
        let mut app = shortcut_app(true, Status::Playing);
        press_shortcut(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyQ]);
        assert!(app.world().resource::<Game>().quit_confirmation);
        assert!(
            app.world()
                .resource::<Messages<bevy::app::AppExit>>()
                .is_empty()
        );
        press_shortcut(&mut app, &[KeyCode::Escape]);
        assert!(!app.world().resource::<Game>().quit_confirmation);
        assert!(app.world().resource::<Game>().board.started);
        assert!(
            app.world()
                .resource::<Messages<bevy::app::AppExit>>()
                .is_empty()
        );
    }

    #[test]
    fn ctrl_q_enter_confirms_and_inactive_games_quit_immediately() {
        let mut app = shortcut_app(true, Status::Playing);
        press_shortcut(&mut app, &[KeyCode::ControlRight, KeyCode::KeyQ]);
        press_shortcut(&mut app, &[KeyCode::Enter]);
        assert!(
            !app.world()
                .resource::<Messages<bevy::app::AppExit>>()
                .is_empty()
        );
        for (started, status) in [
            (false, Status::Playing),
            (true, Status::Won),
            (true, Status::Lost),
        ] {
            let mut app = shortcut_app(started, status);
            press_shortcut(&mut app, &[KeyCode::ControlLeft, KeyCode::KeyQ]);
            assert!(!app.world().resource::<Game>().quit_confirmation);
            assert!(
                !app.world()
                    .resource::<Messages<bevy::app::AppExit>>()
                    .is_empty()
            );
        }
    }

    #[test]
    fn f2_toggles_leaderboard_and_preserves_game_while_request_is_pending() {
        let (online, worker) = online::tests::starting_online();
        let mut app = shortcut_app(true, Status::Playing);
        {
            let mut game = app.world_mut().resource_mut::<Game>();
            game.online = online;
            game.online.session = Some(online::Session {
                token: "b".repeat(64),
                display_name: "Player".into(),
            });
            game.online.dispatch(Operation::Start {
                mode: "easy".into(),
            });
            game.board.flag(0);
        }
        press_shortcut(&mut app, &[KeyCode::F2]);
        assert!(app.world().resource::<Game>().online.show_scores);
        assert!(app.world().resource::<Game>().board.cells[0].flagged);
        press_shortcut(&mut app, &[KeyCode::F2]);
        assert!(!app.world().resource::<Game>().online.show_scores);
        assert!(app.world().resource::<Game>().board.cells[0].flagged);
        worker.join().unwrap();
    }

    #[test]
    fn quit_dialog_blocks_board_actions() {
        let mut app = chord_app();
        app.world_mut().resource_mut::<Game>().quit_confirmation = true;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&TileFlash>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn repeated_middle_click_preserves_original_color_and_skips_flags() {
        let mut app = chord_app();
        {
            let mut game = app.world_mut().resource_mut::<Game>();
            game.board.cells[3].count = 2;
            game.board.flag(0);
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&TileFlash>()
                .iter(app.world())
                .count(),
            2
        );
        // Repeat while the first flash is still visible.
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(201));
        app.update();
        for background in app
            .world_mut()
            .query::<&BackgroundColor>()
            .iter(app.world())
        {
            assert_eq!(background.0, Color::BLACK);
        }
        let game = app.world().resource::<Game>();
        assert!(game.board.cells[0].flagged);
        assert!(!game.board.cells[1].revealed);
    }

    #[test]
    fn middle_click_on_covered_tile_does_not_start_game() {
        let mut app = chord_app();
        app.world_mut().resource_mut::<Game>().board = Board::new(2, 2, 1);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.update();
        assert!(!app.world().resource::<Game>().board.started);
        assert_eq!(
            app.world_mut()
                .query::<&TileFlash>()
                .iter(app.world())
                .count(),
            3
        );
    }

    fn score(player_name: &str, elapsed_ms: u64) -> online::Score {
        online::Score {
            id: None,
            player_name: player_name.into(),
            user_id: Some(1),
            elapsed_ms,
            board: None,
        }
    }

    #[test]
    fn both_leaderboards_show_clickable_winning_board_miniatures() {
        for results in [false, true] {
            let mut game = Game::default();
            let mut score = score("Player", 1000);
            score.id = Some(42);
            score.board = Some(online::WinningBoard {
                rows: 9,
                columns: 9,
                cells: (0..81)
                    .map(|index| board::Cell {
                        mine: index < 10,
                        flagged: index < 10,
                        revealed: index >= 10,
                        count: 1,
                    })
                    .collect(),
            });
            game.online.scores.easy.push(score);
            game.online.scores_loaded = true;
            game.online.show_scores = !results;
            if results {
                let mut over = won_over(1000, Recording::Offline);
                over.mode = Some("easy");
                game.game_over = Some(over);
            }
            let mut app = App::new();
            app.insert_resource(game);
            let mut queue = bevy::ecs::world::CommandQueue::default();
            build_screen(
                &mut Commands::new(&mut queue, app.world()),
                app.world().resource::<Game>(),
            );
            queue.apply(app.world_mut());
            let mut links = app.world_mut().query::<(&ScoreLink, &Children)>();
            let (link, children) = links.single(app.world()).unwrap();
            assert_eq!(link.0, 42);
            assert!(
                children
                    .iter()
                    .any(|child| app.world().get::<ScorePreview>(child).is_some())
            );
            let mut previews = app
                .world_mut()
                .query_filtered::<(&Node, &Children), With<ScorePreview>>();
            let (node, children) = previews.single(app.world()).unwrap();
            assert_eq!(node.grid_template_columns.len(), 9);
            assert_eq!(children.len(), 81);
            assert_eq!(
                app.world_mut()
                    .query_filtered::<Entity, With<ScoreViewport>>()
                    .iter(app.world())
                    .count(),
                1
            );

            if results {
                app.init_resource::<ButtonInput<MouseButton>>()
                    .add_systems(Update, dismiss_game_over_on_click);
                let link_entity = app
                    .world_mut()
                    .query_filtered::<Entity, With<ScoreLink>>()
                    .single(app.world())
                    .unwrap();
                app.world_mut()
                    .entity_mut(link_entity)
                    .insert(Interaction::Pressed);
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(MouseButton::Left);
                app.update();
                assert!(app.world().resource::<Game>().game_over.is_some());
            }
        }
    }

    #[test]
    fn winning_board_link_errors_are_reported_without_opening_a_browser() {
        let mut app = App::new();
        app.init_resource::<Game>()
            .add_systems(Update, open_score_links);
        app.world_mut().spawn((Interaction::Pressed, ScoreLink(0)));
        app.update();
        assert_eq!(
            app.world().resource::<Game>().online.message,
            "Cannot open winning board: The server returned an invalid score ID."
        );
    }

    fn won_over(elapsed_ms: u64, recording: Recording) -> GameOver {
        GameOver {
            won: true,
            reason: String::new(),
            elapsed_ms,
            mode: Some("intermediate"),
            recording,
            lookup: Lookup::Ready,
        }
    }

    #[test]
    fn offline_loss_opens_popup_with_reason_and_escape_dismisses() {
        let mut app = shortcut_app(true, Status::Playing);
        {
            let mut game = app.world_mut().resource_mut::<Game>();
            game.board = Board::new(2, 2, 1);
            game.board.started = true;
            game.board.cells[3].mine = true;
            game.elapsed = 4.25;
            apply_move(&mut game, 3, "reveal");
            let over = game.game_over.as_ref().unwrap();
            assert!(!over.won);
            assert_eq!(
                over.reason,
                "You stepped on a makibishi at row 2, column 2!"
            );
            assert_eq!(over.elapsed_ms, 4250);
            assert_eq!(over.recording, Recording::Lost);
            assert_eq!(over.lookup, Lookup::NotApplicable);
            assert_eq!(
                placement_text(over, &game.online),
                "No leaderboard placement for this game."
            );
        }
        press_shortcut(&mut app, &[KeyCode::F1]);
        assert!(!app.world().resource::<Game>().help_open);
        press_shortcut(&mut app, &[KeyCode::Escape]);
        assert!(app.world().resource::<Game>().game_over.is_none());
    }

    #[test]
    fn misplaced_chord_flag_is_explained() {
        let mut app = chord_app();
        app.world_mut().resource_mut::<Game>().board.flag(1);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.update();
        let game = app.world().resource::<Game>();
        let over = game.game_over.as_ref().unwrap();
        assert_eq!(
            over.reason,
            "Chording row 2, column 2 revealed a makibishi - a flag was misplaced."
        );
    }

    #[test]
    fn custom_win_is_not_recorded_or_ranked() {
        let mut game = Game {
            custom: true,
            board: Board::new(2, 2, 1),
            ..default()
        };
        game.board.started = true;
        game.board.cells[0].mine = true;
        for index in 1..4 {
            game.board.cells[index].count = 1;
        }
        apply_move(&mut game, 3, "reveal");
        apply_move(&mut game, 2, "reveal");
        apply_move(&mut game, 1, "reveal");
        let over = game.game_over.as_ref().unwrap();
        assert!(over.won);
        assert_eq!(over.recording, Recording::Custom);
        assert!(recording_text(over, &game.online).contains("custom games are unranked"));
        assert_eq!(over.lookup, Lookup::NotApplicable);
    }

    #[test]
    fn placement_reports_actual_or_would_be_rank() {
        let mut online = Online::default();
        online.scores.intermediate = vec![score("A", 10_000), score("Bjørn", 20_000)];
        online
            .scores
            .intermediate
            .extend((0..8).map(|i| score("C", 30_000 + i)));
        assert_eq!(
            placement_text(&won_over(15_000, Recording::Offline), &online),
            "Would have placed #2 on Intermediate."
        );
        assert_eq!(
            placement_text(&won_over(20_000, Recording::Failed("x".into())), &online),
            "Would have placed #3 on Intermediate."
        );
        assert_eq!(
            placement_text(&won_over(99_000, Recording::Offline), &online),
            "Would not have made the Intermediate top 10."
        );
        online.session = Some(online::Session {
            token: "t".into(),
            display_name: "Bjørn".into(),
        });
        assert_eq!(
            placement_text(&won_over(20_000, Recording::Recorded), &online),
            "Leaderboard: #2 on Intermediate."
        );
        assert_eq!(
            placement_text(&won_over(99_000, Recording::Recorded), &online),
            "Outside the Intermediate top 10."
        );
    }

    #[test]
    fn online_win_records_score_then_looks_up_placement() {
        let leaderboards = serde_json::json!({
            "easy": [
                {"player_name": "Fast", "user_id": 2, "elapsed_ms": 5000},
                {"player_name": "Bjørn", "user_id": 1, "elapsed_ms": 7500}
            ],
            "intermediate": [],
            "advanced": []
        });
        let score = serde_json::json!({"player_name": "Bjørn", "user_id": 1, "elapsed_ms": 7500});
        let (online, worker) =
            online::tests::scripted_online(vec![(200, score), (200, leaderboards)]);
        let mut game = Game {
            online,
            ..default()
        };
        game.online.session = Some(online::Session {
            token: "token".into(),
            display_name: "Bjørn".into(),
        });
        game.online.game_id = Some("00000000-0000-0000-0000-000000000000".into());
        game.last_move = Some((0, "reveal"));
        let mut cells = vec![board::Cell::default(); 81];
        for (index, cell) in cells.iter_mut().enumerate() {
            cell.mine = index >= 71;
            cell.revealed = index < 71;
        }
        handle_online_reply(
            &mut game,
            Ok(Reply::Board(online::GameView {
                id: "00000000-0000-0000-0000-000000000000".into(),
                mode: "easy".into(),
                rows: 9,
                columns: 9,
                mine_count: 10,
                cells,
                status: "won".into(),
                elapsed_ms: Some(7500),
                clock_started: true,
            })),
        );
        let over = game.game_over.as_ref().unwrap();
        assert_eq!(over.recording, Recording::Pending);
        assert_eq!(over.elapsed_ms, 7500);
        for _ in 0..2 {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let result = loop {
                if let Some(result) = game.online.poll() {
                    break result;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            handle_online_reply(&mut game, result);
            request_placement(&mut game);
        }
        let over = game.game_over.as_ref().unwrap();
        assert_eq!(over.recording, Recording::Recorded);
        assert_eq!(over.lookup, Lookup::Ready);
        assert_eq!(
            recording_text(over, &game.online),
            "Score of 7.50s recorded online."
        );
        assert_eq!(
            placement_text(over, &game.online),
            "Leaderboard: #2 on Beginner."
        );
        assert_eq!(worker.join().unwrap().len(), 2);
    }

    const GAME_ID: &str = "00000000-0000-0000-0000-000000000000";

    fn board_json(flag_first: bool) -> serde_json::Value {
        let cells: Vec<_> = (0..81)
            .map(|index| {
                serde_json::json!({
                    "mine": false, "revealed": false,
                    "flagged": flag_first && index == 0, "count": 0
                })
            })
            .collect();
        serde_json::json!({
            "id": GAME_ID, "mode": "easy", "rows": 9, "columns": 9, "mine_count": 10,
            "cells": cells, "status": "playing", "elapsed_ms": 1000, "clock_started": true
        })
    }

    fn ranked_game(online: Online) -> Game {
        let mut game = Game {
            online,
            ..default()
        };
        game.online.ranked = true;
        game.online.session = Some(online::Session {
            token: "token".into(),
            display_name: "Bjørn".into(),
        });
        game.online.game_id = Some(GAME_ID.into());
        game.board.started = true;
        game
    }

    fn wait_reply(game: &mut Game) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let result = loop {
            if let Some(result) = game.online.poll() {
                break result;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        handle_online_reply(game, result);
    }

    #[test]
    fn failed_move_resyncs_then_resends_the_missing_move() {
        let (online, worker) = online::tests::scripted_online(vec![
            (503, serde_json::json!("Service unavailable")),
            (200, board_json(false)),
            (200, board_json(true)),
        ]);
        let mut game = ranked_game(online);
        apply_move(&mut game, 0, "flag");
        wait_reply(&mut game);
        let lost = game.connection.as_ref().unwrap();
        assert_eq!(lost.attempt, 1);
        assert_eq!(
            lost.retry,
            Retry::Resync {
                id: GAME_ID.into(),
                pending: Some(PendingMove {
                    index: 0,
                    action: "flag",
                    was_flagged: false
                })
            }
        );
        assert!(game.online.needs_refresh);
        dispatch_retry(&mut game);
        wait_reply(&mut game);
        assert!(game.connection.is_none());
        assert!(game.online.busy(), "the unapplied move must be resent");
        wait_reply(&mut game);
        assert!(game.board.cells[0].flagged);
        assert!(!game.online.needs_refresh);
        let requests = worker.join().unwrap();
        assert!(requests[0].starts_with("POST") && requests[0].contains("/move"));
        assert!(requests[1].starts_with("GET"));
        assert!(requests[2].starts_with("POST") && requests[2].contains("/move"));
    }

    #[test]
    fn applied_move_is_not_resent_after_resync() {
        let mut game = ranked_game(Online::default());
        game.connection = Some(ConnectionLost {
            retry: Retry::Resync {
                id: GAME_ID.into(),
                pending: Some(PendingMove {
                    index: 0,
                    action: "flag",
                    was_flagged: false,
                }),
            },
            attempt: 3,
            timer: Timer::from_seconds(1.0, TimerMode::Once),
            error: "down".into(),
        });
        let view: online::GameView = serde_json::from_value(board_json(true)).unwrap();
        handle_online_reply(&mut game, Ok(Reply::Board(view)));
        assert!(game.connection.is_none());
        assert!(!game.online.busy());
        assert!(game.online.message.contains("back in sync"));
    }

    #[test]
    fn repeated_failures_back_off_and_show_popup() {
        let mut game = ranked_game(Online::default());
        let error = || online::ApiError {
            message: "Cannot reach the server".into(),
            unauthorized: false,
            retryable: true,
        };
        game.online.last_game_operation = Some(Operation::Refresh { id: GAME_ID.into() });
        handle_online_reply(&mut game, Err(error()));
        assert_eq!(game.connection.as_ref().unwrap().attempt, 1);
        game.online.last_game_operation = Some(Operation::Refresh { id: GAME_ID.into() });
        handle_online_reply(&mut game, Err(error()));
        let lost = game.connection.as_ref().unwrap();
        assert_eq!(lost.attempt, 2);
        assert_eq!(lost.timer.duration().as_secs_f32(), 2.0);
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut world = World::new();
        build_screen(&mut Commands::new(&mut queue, &world), &game);
        queue.apply(&mut world);
        assert_eq!(world.query::<&RetryText>().iter(&world).count(), 1);
        assert_eq!(retry_delay(99), 15.0);
    }

    #[test]
    fn escape_on_connection_popup_abandons_ranked_game() {
        let mut app = shortcut_app(true, Status::Playing);
        {
            let mut game = app.world_mut().resource_mut::<Game>();
            game.online.ranked = true;
            game.online.game_id = Some(GAME_ID.into());
            game.online.needs_refresh = true;
            game.connection = Some(ConnectionLost {
                retry: Retry::Resync {
                    id: GAME_ID.into(),
                    pending: None,
                },
                attempt: 2,
                timer: Timer::from_seconds(2.0, TimerMode::Once),
                error: "down".into(),
            });
        }
        press_shortcut(&mut app, &[KeyCode::F1]);
        assert!(!app.world().resource::<Game>().help_open);
        press_shortcut(&mut app, &[KeyCode::Escape]);
        let game = app.world().resource::<Game>();
        assert!(game.connection.is_none());
        assert!(!game.online.ranked);
        assert!(game.online.game_id.is_none());
        assert!(!game.online.needs_refresh);
        assert!(!game.board.started);
    }

    #[test]
    fn submit_retry_keeps_score_pending_until_player_stops() {
        let mut game = ranked_game(Online::default());
        game.game_over = Some(won_over(5000, Recording::Pending));
        game.online.last_game_operation = Some(Operation::Submit { id: GAME_ID.into() });
        handle_online_reply(
            &mut game,
            Err(online::ApiError {
                message: "Server error 502".into(),
                unauthorized: false,
                retryable: true,
            }),
        );
        assert_eq!(
            game.game_over.as_ref().unwrap().recording,
            Recording::Pending
        );
        stop_retrying(&mut game);
        assert!(game.online.ranked);
        assert_eq!(
            game.game_over.as_ref().unwrap().recording,
            Recording::Failed("Server error 502".into())
        );
    }
}
