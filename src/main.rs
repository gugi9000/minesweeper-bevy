mod board;
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

#[derive(Resource)]
struct Game {
    board: Board,
    elapsed: f32,
    flag_mode: bool,
    custom: bool,
    custom_rows: usize,
    custom_columns: usize,
    custom_mines: usize,
    online: Online,
    quit_confirmation: bool,
    help_open: bool,
    zoomed: bool,
    board_scroll: Vec2,
    last_move: Option<(usize, &'static str)>,
    game_over: Option<GameOver>,
    connection: Option<ConnectionLost>,
    tile_images: Option<TileImages>,
    banner: Handle<Image>,
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
            online: Online::default(),
            quit_confirmation: false,
            help_open: false,
            zoomed: false,
            board_scroll: Vec2::ZERO,
            last_move: None,
            game_over: None,
            connection: None,
            tile_images: None,
            banner: Handle::default(),
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
struct BoardViewport;
#[derive(Component, Clone, Copy)]
enum Action {
    Beginner,
    Intermediate,
    Advanced,
    Custom,
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
    CloseGameOver,
    RetryNow,
    StopRetrying,
    Zoom,
}

#[derive(Component)]
struct RetryText;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: env!("CARGO_MANIFEST_DIR").into(),
                ..default()
            })
            .set(ImagePlugin::default_nearest())
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Minesweeper".into(),
                    resolution: (1200, 950).into(),
                    ..default()
                }),
                ..default()
            }),
    )
    .insert_resource(ClearColor(Color::srgb(0.07, 0.09, 0.13)))
    .init_resource::<Game>()
    .add_systems(Startup, setup)
    .add_systems(
        Update,
        (
            login_input,
            keyboard,
            controls,
            tiles,
            update_flashes,
            poll_online,
            retry_connection,
            update_status,
            scroll_board,
        )
            .chain(),
    );
    #[cfg(target_os = "windows")]
    app.world_mut()
        .resource_mut::<Assets<Font>>()
        .insert(bevy::asset::AssetId::default(), windows_ui_font()?)?;
    app.run();
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_ui_font() -> Result<Font, Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(
        std::env::var_os("WINDIR")
            .ok_or("WINDIR is unavailable; cannot locate the Windows UI font")?,
    )
    .join("Fonts")
    .join("segoeui.ttf");
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("Cannot read UI font {}: {error}", path.display()))?;
    Font::try_from_bytes(bytes)
        .map_err(|error| format!("Cannot load UI font {}: {error}", path.display()).into())
}

fn setup(mut commands: Commands, mut game: ResMut<Game>, asset_server: Res<AssetServer>) {
    commands.spawn(Camera2d);
    game.banner = asset_server.load("gfx/Makibishi Sweeper.png");
    game.tile_images = Some(TileImages {
        covered: std::array::from_fn(|variant| {
            asset_server.load(format!("gfx/tile-{variant}.png"))
        }),
        flagged: asset_server.load("gfx/sashimono.png"),
        mine: asset_server.load("gfx/makibishi.png"),
    });
    game.online.dispatch(Operation::Restore);
    build_screen(&mut commands, &game);
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

fn build_screen(commands: &mut Commands, game: &Game) {
    commands
        .spawn((
            Screen,
            NetworkStatusSlot,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(0),
                left: px(0),
                width: percent(100),
                height: px(84),
                min_height: px(84),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                flex_shrink: 0.0,
                padding: UiRect::axes(px(16), px(8)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgb(0.12, 0.17, 0.23)),
            GlobalZIndex(10),
        ))
        .with_children(|parent| {
            parent
                .spawn(Node {
                    column_gap: px(12),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|parent| {
                    parent.spawn((label("", 16.0), StatusText));
                    button(
                        parent,
                        if game.flag_mode {
                            "(F) Flag mode: ON"
                        } else {
                            "(F) Flag mode: OFF"
                        },
                        Action::FlagMode,
                    );
                    button(parent, "Fullscreen (F11)", Action::Fullscreen);
                    button(parent, "F1 - How to play", Action::Help);
                    button(
                        parent,
                        if game.zoomed {
                            "Zoom 2x (F5)"
                        } else {
                            "Zoom 1x (F5)"
                        },
                        Action::Zoom,
                    );
                });
            parent.spawn(label(&game.online.message, 16.0));
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
                    parent.spawn(label("Reveal all safe tiles without hitting a mine.\nNumbers count mines in the eight neighboring tiles.\nLeft click: reveal. Right click: flag.\nClick a number or middle / left+right click: chord when flags match.\nOtherwise, middle click flashes uncleared neighbors.\nF: flag mode. N: new game. F2: online leaderboard.\nF5: toggle 2x board zoom. Wheel: scroll; Shift+wheel: sideways.\nF11: fullscreen. Ctrl+Q: quit (confirmation during active games).\nOffline and Custom games are unranked.", 18.0));
                    button(parent, "Close (F1 / Esc)", Action::CloseHelp);
                });
        });
    }
    if let Some(over) = game.game_over.as_ref().filter(|_| !game.quit_confirmation) {
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
                            flex_direction: FlexDirection::Column,
                            row_gap: px(12),
                            padding: UiRect::all(px(24)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.12, 0.17, 0.23)),
                    ))
                    .with_children(|parent| {
                        let (title, color) = if over.won {
                            ("You won!", Color::srgb(0.35, 0.85, 0.45))
                        } else {
                            ("You lost", Color::srgb(1.0, 0.4, 0.4))
                        };
                        parent.spawn(colored_label(title, 30.0, color));
                        parent.spawn(label(&over.reason, 18.0));
                        parent.spawn(label(
                            format!("Time: {:.2}s", over.elapsed_ms as f64 / 1000.0),
                            18.0,
                        ));
                        parent.spawn(label(recording_text(over, &game.online), 18.0));
                        parent.spawn(label(placement_text(over, &game.online), 18.0));
                        button(parent, "Close (Esc)", Action::CloseGameOver);
                    });
            });
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
                        parent.spawn(label("A game is in progress. Quit Minesweeper?", 24.0));
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
    commands
        .spawn((
            Screen,
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(12),
                padding: UiRect {
                    bottom: px(100),
                    ..UiRect::all(px(16))
                },
                overflow: Overflow::scroll_y(),
                ..default()
            },
        ))
        .with_children(|parent| {
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
            let account = game.online.session.as_ref().map_or(
                "Playing anonymously. Sign in for online ranked games.".to_string(),
                |session| format!("Signed in as {}", session.display_name),
            );
            parent.spawn(label(account, 16.0));
            parent
                .spawn(Node {
                    column_gap: px(8),
                    ..default()
                })
                .with_children(|parent| {
                    if game.online.session.is_some() {
                        button(parent, "Sign out", Action::Logout);
                        button(
                            parent,
                            if game.online.ranked {
                                "Online mode: ON"
                            } else {
                                "Online mode: OFF"
                            },
                            Action::Ranked,
                        );
                    } else {
                        button(parent, "Sign in", Action::Login);
                    }
                    button(parent, "Online leaderboard (F2)", Action::Scores);
                    if game.online.needs_refresh && !game.online.busy() {
                        button(parent, "Refresh board", Action::RefreshBoard);
                    }
                    if game.online.game_id.is_some()
                        && game.board.status == Status::Won
                        && !game.online.submitted
                    {
                        button(parent, "Submit score", Action::SubmitScore);
                    }
                });
            if game.online.login_open {
                parent.spawn(label(
                    "Sign in with your wasd.dk account (Tab switches fields; Enter signs in).",
                    18.0,
                ));
                parent
                    .spawn(Node {
                        column_gap: px(8),
                        ..default()
                    })
                    .with_children(|parent| {
                        button(
                            parent,
                            &format!(
                                "{}Username: {}",
                                if game.online.password_focus { "" } else { "> " },
                                game.online.username
                            ),
                            Action::Username,
                        );
                        button(
                            parent,
                            &format!(
                                "{}Password: {}",
                                if game.online.password_focus { "> " } else { "" },
                                "*".repeat(game.online.password.chars().count())
                            ),
                            Action::Password,
                        );
                    });
                parent
                    .spawn(Node {
                        column_gap: px(8),
                        ..default()
                    })
                    .with_children(|parent| {
                        button(
                            parent,
                            if game.online.remember {
                                "Remember login: YES"
                            } else {
                                "Remember login: NO"
                            },
                            Action::Remember,
                        );
                        button(parent, "Sign in", Action::SignIn);
                        button(parent, "Cancel", Action::CancelLogin);
                    });
                return;
            }
            if game.online.show_scores {
                parent.spawn(label(
                    "Top 10 server-managed games per difficulty (* = guest account not verified)",
                    18.0,
                ));
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
                                        parent.spawn(label(
                                            format!(
                                                "{}. {}{} - {:.2}s",
                                                rank + 1,
                                                score.player_name,
                                                if score.user_id.is_none() { " *" } else { "" },
                                                score.elapsed_ms as f64 / 1000.0
                                            ),
                                            16.0,
                                        ));
                                    }
                                });
                        }
                    });
                button(parent, "Back to game", Action::BackToGame);
                return;
            }
            parent
                .spawn(Node {
                    column_gap: px(8),
                    ..default()
                })
                .with_children(|parent| {
                    button(parent, "Beginner", Action::Beginner);
                    button(parent, "Intermediate", Action::Intermediate);
                    button(parent, "Advanced", Action::Advanced);
                    button(parent, "Custom", Action::Custom);
                    button(parent, "New game (N)", Action::NewGame);
                });
            if game.custom {
                parent.spawn(label(
                    format!(
                        "Custom: {} rows, {} columns, {} mines",
                        game.custom_rows, game.custom_columns, game.custom_mines
                    ),
                    18.0,
                ));
                parent
                    .spawn(Node {
                        column_gap: px(8),
                        ..default()
                    })
                    .with_children(|parent| {
                        button(parent, "Rows -", Action::Rows(-1));
                        button(parent, "Rows +", Action::Rows(1));
                        button(parent, "Columns -", Action::Columns(-1));
                        button(parent, "Columns +", Action::Columns(1));
                        button(parent, "Mines -", Action::Mines(-1));
                        button(parent, "Mines +", Action::Mines(1));
                    });
            }
            let scale = if game.zoomed { 2.0 } else { 1.0 };
            let tile_size = board_tile_size(game);
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
                        .spawn(Node {
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
                        })
                        .with_children(|parent| {
                            for (index, cell) in game.board.cells.iter().enumerate() {
                                let show_mine = cell.mine
                                    && (cell.revealed || game.board.status == Status::Lost);
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

fn board_tile_size(game: &Game) -> f32 {
    (1000.0 / game.board.columns as f32)
        .min(580.0 / game.board.rows as f32)
        .min(34.0)
        * if game.zoomed { 2.0 } else { 1.0 }
}

fn scroll_board(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    mut game: ResMut<Game>,
    mut viewport: Query<(&ComputedNode, &mut ScrollPosition), With<BoardViewport>>,
) {
    for event in wheel.read() {
        if game.help_open
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
        if keys.just_pressed(KeyCode::Escape) {
            game.game_over = None;
            rebuild(&mut commands, &screen, &game);
        }
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
    if keys.just_pressed(KeyCode::F11) {
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
        if game.game_over.is_some() && !matches!(action, Action::CloseGameOver) {
            continue;
        }
        if game.connection.is_some() && !matches!(action, Action::RetryNow | Action::StopRetrying) {
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
                    | Action::CloseGameOver
                    | Action::Zoom
                    | Action::FlagMode
            )
        {
            continue;
        }
        match action {
            Action::Help => game.help_open = !game.help_open,
            Action::CloseHelp => game.help_open = false,
            Action::CloseGameOver => game.game_over = None,
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
                game.custom = true;
                new_board(&mut game);
            }
            Action::NewGame => new_board(&mut game),
            Action::FlagMode => game.flag_mode = !game.flag_mode,
            Action::Rows(delta) => {
                game.custom_rows = (game.custom_rows as i32 + delta).clamp(5, 24) as usize
            }
            Action::Columns(delta) => {
                game.custom_columns = (game.custom_columns as i32 + delta).clamp(5, 30) as usize
            }
            Action::Mines(delta) => {
                game.custom_mines = (game.custom_mines as i32 + delta).max(1) as usize
            }
            Action::Login => game.online.login_open = true,
            Action::Username => game.online.password_focus = false,
            Action::Password => game.online.password_focus = true,
            Action::Remember => game.online.remember = !game.online.remember,
            Action::SignIn => sign_in(&mut game.online),
            Action::CancelLogin => {
                game.online.login_open = false;
                game.online.password.clear();
            }
            Action::Logout => {
                game.online.dispatch(Operation::Logout);
            }
            Action::Ranked => {
                game.online.ranked = !game.online.ranked;
                new_board(&mut game);
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
        game.custom_mines = game
            .custom_mines
            .min(game.custom_rows * game.custom_columns - 1);
        rebuild(&mut commands, &screen, &game);
        break;
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
        || game.help_open
        || game.quit_confirmation
        || game.game_over.is_some()
        || game.connection.is_some()
        || game.online.login_open
        || game.online.show_scores
        || (game.online.ranked
            && !game.custom
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
            let elapsed_ms = (game.elapsed * 1000.0).round() as u64;
            finish_game(game, elapsed_ms);
        }
    }
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
    let reason = if won {
        "Every safe tile was cleared.".to_string()
    } else {
        match game.last_move {
            Some((index, action)) => {
                let row = index / game.board.columns + 1;
                let column = index % game.board.columns + 1;
                if action == "chord" {
                    format!(
                        "Chording row {row}, column {column} opened a mine - a flag was misplaced."
                    )
                } else {
                    format!("You revealed a mine at row {row}, column {column}.")
                }
            }
            None => "A mine was revealed.".to_string(),
        }
    };
    let mode = leaderboard_mode(game);
    let recording = if !won {
        Recording::Lost
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
        lookup: if won && mode.is_some() {
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
    match &over.recording {
        Recording::Lost => "Score not recorded: only cleared boards are scored.".into(),
        Recording::Custom => "Score not recorded: custom games are unranked.".into(),
        Recording::Offline if online.session.is_none() => {
            "Score not recorded: offline games are unranked. Log in to play ranked.".into()
        }
        Recording::Offline => {
            "Score not recorded: offline games are unranked. Turn on ranked play to compete.".into()
        }
        Recording::Pending => "Recording score online...".into(),
        Recording::Recorded => "Score recorded online.".into(),
        Recording::Failed(error) => {
            format!("Score not recorded online: {error} Use Submit score to retry.")
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
    let scores = match mode {
        "intermediate" => &online.scores.intermediate,
        "advanced" => &online.scores.advanced,
        _ => &online.scores.easy,
    };
    if over.recording == Recording::Recorded {
        let player = online.session.as_ref().map(|s| s.display_name.as_str());
        let place = scores.iter().take(LEADERBOARD_SIZE).rposition(|s| {
            s.elapsed_ms == over.elapsed_ms && Some(s.player_name.as_str()) == player
        });
        match place {
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
                game.online.login_open = true;
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
                        Ok((id, board, elapsed)) => {
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
) {
    if game.board.started && game.board.status == Status::Playing {
        game.elapsed += time.delta_secs();
    }
    let flags = game.board.cells.iter().filter(|cell| cell.flagged).count();
    let state = match game.board.status {
        Status::Playing => "Good luck!",
        Status::Won => "Board cleared!",
        Status::Lost => "Mine hit!",
    };
    for mut text in &mut text {
        **text = format!(
            "Mines: {}   Time: {:.1}s   {}   {}",
            game.board.mines as isize - flags as isize,
            game.elapsed,
            state,
            if game.online.game_id.is_some() {
                "Online ranked"
            } else {
                "Offline / unranked"
            }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert_eq!(slot.height, px(84));
            assert_eq!(slot.min_height, px(84));
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
        assert!(labels.iter().any(|text| text == "> Password: ************"));
        assert!(labels.iter().all(|text| !text.contains("test-passwor")));
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
        let (image, node) = query.single(app.world()).unwrap();
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
    fn stats_and_controls_are_children_of_bottom_status_bar() {
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
        assert_eq!(app.world().get::<ChildOf>(row).unwrap().parent(), status);
        for (entity, action) in app
            .world_mut()
            .query::<(Entity, &Action)>()
            .iter(app.world())
        {
            if matches!(action, Action::FlagMode | Action::Fullscreen) {
                assert_eq!(app.world().get::<ChildOf>(entity).unwrap().parent(), row);
            }
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
            player_name: player_name.into(),
            user_id: Some(1),
            elapsed_ms,
        }
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
            assert_eq!(over.reason, "You revealed a mine at row 2, column 2.");
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
            "Chording row 2, column 2 opened a mine - a flag was misplaced."
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
        assert_eq!(recording_text(over, &game.online), "Score recorded online.");
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
