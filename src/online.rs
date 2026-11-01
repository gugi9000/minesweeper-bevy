use std::{
    sync::{
        Mutex,
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};

use reqwest::{
    Url,
    blocking::{Client, RequestBuilder},
};
use serde::{Deserialize, de::DeserializeOwned};

use crate::board::{Board, Cell, Status};

#[derive(Deserialize)]
pub struct Session {
    pub token: String,
    pub display_name: String,
}

#[derive(Deserialize)]
struct Profile {
    display_name: String,
}

#[derive(Deserialize)]
pub struct GameView {
    pub id: String,
    pub mode: String,
    pub rows: usize,
    pub columns: usize,
    pub mine_count: usize,
    pub cells: Vec<Cell>,
    pub status: String,
    pub elapsed_ms: Option<u64>,
    pub clock_started: bool,
}

impl GameView {
    pub fn into_board(self) -> Result<(String, Board, Option<f32>), String> {
        if !(5..=24).contains(&self.rows)
            || !(5..=30).contains(&self.columns)
            || self.cells.len() != self.rows * self.columns
            || self.mine_count == 0
            || self.mine_count >= self.cells.len()
            || self.cells.iter().any(|cell| cell.count > 8)
            || uuid_like(&self.id).is_none()
            || !matches!(
                self.mode.as_str(),
                "easy" | "intermediate" | "advanced" | "custom"
            )
        {
            return Err("The server returned an invalid board.".into());
        }
        let status = match self.status.as_str() {
            "playing" => Status::Playing,
            "won" => Status::Won,
            "lost" => Status::Lost,
            _ => return Err("The server returned an unknown game status.".into()),
        };
        Ok((
            self.id,
            Board {
                rows: self.rows,
                columns: self.columns,
                mines: self.mine_count,
                cells: self.cells,
                status,
                started: self.clock_started,
            },
            self.elapsed_ms.map(|ms| ms as f32 / 1000.0),
        ))
    }
}

fn uuid_like(id: &str) -> Option<&str> {
    (id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')).then_some(id)
}

#[derive(Deserialize)]
pub struct Score {
    pub player_name: String,
    pub user_id: Option<i32>,
    pub elapsed_ms: u64,
}

#[derive(Default, Deserialize)]
pub struct Leaderboards {
    pub easy: Vec<Score>,
    pub intermediate: Vec<Score>,
    pub advanced: Vec<Score>,
}

#[derive(Clone)]
pub enum Operation {
    Restore,
    Login {
        username: String,
        password: String,
        remember: bool,
    },
    Logout,
    Scores,
    Start {
        mode: String,
    },
    Move {
        id: String,
        action: &'static str,
        row: usize,
        column: usize,
    },
    Refresh {
        id: String,
    },
    Submit {
        id: String,
    },
}

pub enum Reply {
    Restored(Option<Session>),
    LoggedIn(Session, Option<String>),
    LoggedOut(Result<(), String>),
    Scores(Leaderboards),
    Board(GameView),
    Submitted,
}

pub struct ApiError {
    pub message: String,
    pub unauthorized: bool,
    /// Transport failures and temporary server errors that may succeed when repeated.
    pub retryable: bool,
}

impl From<String> for ApiError {
    fn from(message: String) -> Self {
        Self {
            message,
            unauthorized: false,
            retryable: false,
        }
    }
}

#[derive(Clone)]
struct Api {
    client: Client,
    base: String,
    #[cfg(test)]
    credential_service: String,
}

impl Api {
    fn new(base: &str) -> Result<Self, String> {
        let url = Url::parse(base).map_err(|error| format!("Invalid server URL: {error}"))?;
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if (url.scheme() != "https" && !(url.scheme() == "http" && local))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(
                "Server URL must be an HTTPS origin (HTTP is allowed only on localhost).".into(),
            );
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| format!("Cannot initialize networking: {error}"))?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').into(),
            #[cfg(test)]
            credential_service: format!("minesweeper-bevy-test-{:032x}", rand::random::<u128>()),
        })
    }

    fn entry(&self) -> Result<keyring::Entry, String> {
        #[cfg(target_os = "windows")]
        {
            #[cfg(not(test))]
            let service = "minesweeper-bevy";
            #[cfg(test)]
            let service = &self.credential_service;
            keyring::Entry::new(service, &self.base)
                .map_err(|error| format!("Credential store unavailable: {error}"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            Err("Remembered login is currently supported only on Windows.".into())
        }
    }

    fn forget(&self) -> Result<(), String> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("Could not remove remembered login: {error}")),
        }
    }

    fn get(&self, path: &str, token: Option<&str>) -> RequestBuilder {
        let request = self.client.get(format!("{}{path}", self.base));
        if let Some(token) = token {
            request.bearer_auth(token)
        } else {
            request
        }
    }

    fn post(&self, path: &str, token: Option<&str>) -> RequestBuilder {
        let request = self.client.post(format!("{}{path}", self.base));
        if let Some(token) = token {
            request.bearer_auth(token)
        } else {
            request
        }
    }

    fn execute(&self, operation: Operation, token: Option<&str>) -> Result<Reply, ApiError> {
        let required = || {
            token.ok_or_else(|| ApiError {
                message: "Sign in before starting an online game.".into(),
                unauthorized: true,
                retryable: false,
            })
        };
        match operation {
            Operation::Restore => {
                let token = match self.entry()?.get_password() {
                    Ok(token) => token,
                    Err(keyring::Error::NoEntry) => return Ok(Reply::Restored(None)),
                    Err(error) => {
                        return Err(format!("Cannot read remembered login: {error}").into());
                    }
                };
                let profile: Profile = match response(self.get("/api/player/me", Some(&token))) {
                    Ok(profile) => profile,
                    Err(mut error) => {
                        if error.unauthorized
                            && let Err(cleanup) = self.forget()
                        {
                            error.message.push_str(&format!(" {cleanup}"));
                        }
                        return Err(error);
                    }
                };
                Ok(Reply::Restored(Some(Session {
                    token,
                    display_name: profile.display_name,
                })))
            }
            Operation::Login {
                username,
                password,
                remember,
            } => {
                let session: Session = response(self.post("/api/player/login", None).json(
                    &serde_json::json!({
                        "username": username, "password": password, "remember": remember,
                    }),
                ))?;
                if session.token.len() != 64
                    || !session.token.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err("Server returned an invalid session token."
                        .to_string()
                        .into());
                }
                let storage = if remember {
                    self.entry().and_then(|entry| {
                        entry.set_password(&session.token).map_err(|error| {
                            format!("Login succeeded, but remembering login failed: {error}")
                        })
                    })
                } else {
                    self.forget()
                };
                Ok(Reply::LoggedIn(session, storage.err()))
            }
            Operation::Logout => {
                let removal = self.forget();
                let revocation: Result<bool, ApiError> =
                    response(self.post("/api/player/logout", Some(required()?)));
                let errors: Vec<_> = removal
                    .err()
                    .into_iter()
                    .chain(
                        revocation
                            .err()
                            .map(|e| format!("Server session revocation failed: {}", e.message)),
                    )
                    .collect();
                Ok(Reply::LoggedOut(if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors.join(" "))
                }))
            }
            Operation::Scores => Ok(Reply::Scores(response(
                self.get("/games/minesweeper/api/scores", None),
            )?)),
            Operation::Start { mode } => Ok(Reply::Board(response(
                self.post("/api/player/minesweeper/games", Some(required()?))
                    .json(&serde_json::json!({ "mode": mode })),
            )?)),
            Operation::Move {
                id,
                action,
                row,
                column,
            } => {
                if uuid_like(&id).is_none() {
                    return Err("Invalid game ID.".to_string().into());
                }
                Ok(Reply::Board(response(
                    self.post(
                        &format!("/api/player/minesweeper/games/{id}/move"),
                        Some(required()?),
                    )
                    .json(&serde_json::json!({ "action": action, "row": row, "column": column })),
                )?))
            }
            Operation::Refresh { id } => {
                if uuid_like(&id).is_none() {
                    return Err("Invalid game ID.".to_string().into());
                }
                Ok(Reply::Board(response(self.get(
                    &format!("/api/player/minesweeper/games/{id}"),
                    Some(required()?),
                ))?))
            }
            Operation::Submit { id } => {
                if uuid_like(&id).is_none() {
                    return Err("Invalid game ID.".to_string().into());
                }
                let _: Score = response(
                    self.post(
                        &format!("/api/player/minesweeper/games/{id}/score"),
                        Some(required()?),
                    )
                    .json(&serde_json::json!({})),
                )?;
                Ok(Reply::Submitted)
            }
        }
    }
}

fn response<T: DeserializeOwned>(request: RequestBuilder) -> Result<T, ApiError> {
    let response = request.send().map_err(|error| ApiError {
        message: format!("Cannot reach the server: {}", error.without_url()),
        unauthorized: false,
        retryable: true,
    })?;
    let status = response.status();
    if !status.is_success() {
        let retryable = status.is_server_error()
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status == reqwest::StatusCode::REQUEST_TIMEOUT;
        let message = response.text().map_err(|error| ApiError {
            message: format!("Cannot read server error: {}", error.without_url()),
            unauthorized: false,
            retryable,
        })?;
        return Err(ApiError {
            message: format!(
                "Server error {status}: {}",
                message.chars().take(300).collect::<String>()
            ),
            unauthorized: status == reqwest::StatusCode::UNAUTHORIZED,
            retryable,
        });
    }
    response
        .json()
        .map_err(|error| ApiError::from(format!("Invalid server response: {error}")))
}

pub struct Online {
    api: Result<Api, String>,
    pending: Option<Mutex<Receiver<Result<Reply, ApiError>>>>,
    pub session: Option<Session>,
    pub game_id: Option<String>,
    pub ranked: bool,
    pub message: String,
    pub scores: Leaderboards,
    pub scores_loaded: bool,
    pub show_scores: bool,
    pub deferred_scores: bool,
    pub login_open: bool,
    pub username: String,
    pub password: String,
    pub password_focus: bool,
    pub remember: bool,
    pub submitted: bool,
    pub needs_refresh: bool,
    /// Last dispatched game request (never login), kept so failures can be retried.
    pub last_game_operation: Option<Operation>,
}

impl Default for Online {
    fn default() -> Self {
        let api = match std::env::var("MINESWEEPER_SERVER") {
            Ok(base) => Api::new(&base),
            Err(std::env::VarError::NotPresent) => Api::new("https://wasd.dk"),
            Err(error) => Err(format!("Invalid MINESWEEPER_SERVER: {error}")),
        };
        Self {
            message: api.as_ref().err().cloned().unwrap_or_default(),
            api,
            pending: None,
            session: None,
            game_id: None,
            ranked: false,
            scores: Leaderboards::default(),
            scores_loaded: false,
            show_scores: false,
            deferred_scores: false,
            login_open: false,
            username: String::new(),
            password: String::new(),
            password_focus: false,
            remember: false,
            submitted: false,
            needs_refresh: false,
            last_game_operation: None,
        }
    }
}

impl Online {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn dispatch(&mut self, operation: Operation) {
        if self.busy() {
            return;
        }
        let api = match &self.api {
            Ok(api) => api.clone(),
            Err(error) => {
                self.message.clone_from(error);
                return;
            }
        };
        let token = self.session.as_ref().map(|s| s.token.clone());
        let game_operation = matches!(
            operation,
            Operation::Start { .. }
                | Operation::Move { .. }
                | Operation::Refresh { .. }
                | Operation::Submit { .. }
        )
        .then(|| operation.clone());
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("minesweeper-network".into())
            .spawn(move || {
                let result = api.execute(operation, token.as_deref());
                // A closed receiver means the app has exited.
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.pending = Some(Mutex::new(receiver));
                self.last_game_operation = game_operation;
                self.message = "Contacting server...".into();
            }
            Err(error) => self.message = format!("Cannot start network worker: {error}"),
        }
    }

    pub fn poll(&mut self) -> Option<Result<Reply, ApiError>> {
        let result = match self.pending.as_ref()?.lock() {
            Ok(receiver) => match receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    Err("Network worker stopped unexpectedly.".to_string().into())
                }
            },
            Err(error) => Err(format!("Cannot read network result: {error}").into()),
        };
        self.pending = None;
        Some(result)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread::JoinHandle,
        time::Instant,
    };

    fn mock_api(replies: Vec<(u16, serde_json::Value)>) -> (Api, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = Api::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        listener.set_nonblocking(true).unwrap();
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "Expected request never arrived");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 2048];
                    let size = stream.read(&mut buffer).unwrap();
                    assert!(size > 0);
                    bytes.extend_from_slice(&buffer[..size]);
                    assert!(bytes.len() < 65_536);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = std::str::from_utf8(&bytes[..end]).unwrap();
                        let length = header
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
                let body = body.to_string();
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (api, worker)
    }

    pub(crate) fn scripted_online(
        replies: Vec<(u16, serde_json::Value)>,
    ) -> (Online, JoinHandle<Vec<String>>) {
        let (api, worker) = mock_api(replies);
        (
            Online {
                api: Ok(api),
                ..Online::default()
            },
            worker,
        )
    }

    pub(crate) fn starting_online() -> (Online, JoinHandle<Vec<String>>) {
        let (api, worker) = mock_api(vec![(200, view())]);
        (
            Online {
                api: Ok(api),
                ..Online::default()
            },
            worker,
        )
    }

    fn execute(api: &Api, operation: Operation, token: Option<&str>) -> Reply {
        api.execute(operation, token)
            .unwrap_or_else(|error| panic!("{}", error.message))
    }

    #[cfg(target_os = "windows")]
    struct CredentialCleanup(Api);

    #[cfg(target_os = "windows")]
    impl Drop for CredentialCleanup {
        fn drop(&mut self) {
            if let Err(error) = self.0.forget() {
                eprintln!("{error}");
            }
        }
    }

    fn view() -> serde_json::Value {
        serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000", "mode": "easy",
            "rows": 9, "columns": 9, "mine_count": 10,
            "cells": vec![serde_json::json!({"mine": false, "revealed": false, "flagged": false, "count": 0}); 81],
            "status": "playing", "elapsed_ms": null, "clock_started": false
        })
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn http_protocol_and_windows_remembered_login_round_trip() {
        let token = "b".repeat(64);
        let id = "00000000-0000-0000-0000-000000000000";
        let score = serde_json::json!({"player_name":"Player","user_id":1,"elapsed_ms":1250});
        let (api, worker) = mock_api(vec![
            (
                200,
                serde_json::json!({"token":token,"display_name":"Player","expires_at":12345}),
            ),
            (200, serde_json::json!({"display_name":"Renamed"})),
            (200, view()),
            (200, view()),
            (200, view()),
            (200, score.clone()),
            (
                200,
                serde_json::json!({"easy":[score],"intermediate":[],"advanced":[]}),
            ),
            (200, serde_json::json!(true)),
        ]);
        let _cleanup = CredentialCleanup(api.clone());
        let Reply::LoggedIn(session, warning) = execute(
            &api,
            Operation::Login {
                username: "test-player".into(),
                password: "test-password".into(),
                remember: true,
            },
            None,
        ) else {
            panic!("Expected login");
        };
        assert!(warning.is_none());
        assert_eq!(api.entry().unwrap().get_password().unwrap(), token);
        let Reply::Restored(Some(restored)) = execute(&api, Operation::Restore, None) else {
            panic!("Expected restored login");
        };
        assert_eq!(restored.display_name, "Renamed");
        assert!(matches!(
            execute(
                &api,
                Operation::Start {
                    mode: "easy".into()
                },
                Some(&session.token)
            ),
            Reply::Board(_)
        ));
        assert!(matches!(
            execute(
                &api,
                Operation::Move {
                    id: id.into(),
                    action: "chord",
                    row: 2,
                    column: 3
                },
                Some(&token)
            ),
            Reply::Board(_)
        ));
        assert!(matches!(
            execute(&api, Operation::Refresh { id: id.into() }, Some(&token)),
            Reply::Board(_)
        ));
        assert!(matches!(
            execute(&api, Operation::Submit { id: id.into() }, Some(&token)),
            Reply::Submitted
        ));
        let Reply::Scores(scores) = execute(&api, Operation::Scores, None) else {
            panic!("Expected scores");
        };
        assert_eq!(scores.easy[0].elapsed_ms, 1250);
        assert!(matches!(
            execute(&api, Operation::Logout, Some(&token)),
            Reply::LoggedOut(Ok(()))
        ));
        let stored = api.entry().unwrap().get_password();
        assert!(
            matches!(stored, Err(keyring::Error::NoEntry)),
            "Unexpected credential result after logout: {:?}",
            stored.as_ref().map(|_| "still present")
        );
        let requests = worker.join().unwrap();
        assert!(requests[0].starts_with("POST /api/player/login "));
        assert!(!requests[0].to_lowercase().contains("authorization:"));
        let body: serde_json::Value =
            serde_json::from_str(requests[3].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"action":"chord","row":2,"column":3})
        );
        for index in [1, 2, 3, 4, 5, 7] {
            assert!(
                requests[index]
                    .to_lowercase()
                    .contains(&format!("authorization: bearer {token}"))
            );
            assert!(!requests[index].contains("test-password"));
        }
        assert!(!requests[6].to_lowercase().contains("authorization:"));
    }

    #[test]
    fn authentication_and_server_errors_are_not_successes() {
        let (api, worker) = mock_api(vec![
            (401, serde_json::json!("Session expired")),
            (409, serde_json::json!("Board changed")),
            (302, serde_json::json!("Redirect refused")),
        ]);
        for (index, expected_unauthorized) in [true, false, false].into_iter().enumerate() {
            let result = api.execute(
                Operation::Refresh {
                    id: "00000000-0000-0000-0000-000000000000".into(),
                },
                Some(&"b".repeat(64)),
            );
            let Err(error) = result else {
                panic!("Expected explicit error {index}");
            };
            assert_eq!(error.unauthorized, expected_unauthorized);
            assert!(error.message.starts_with("Server error"));
        }
        assert_eq!(worker.join().unwrap().len(), 3);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn expired_remembered_token_is_removed_from_windows_credentials() {
        let (api, worker) = mock_api(vec![(401, serde_json::json!("Session expired"))]);
        let _cleanup = CredentialCleanup(api.clone());
        api.entry().unwrap().set_password(&"b".repeat(64)).unwrap();
        let Err(error) = api.execute(Operation::Restore, None) else {
            panic!("Expected expired session");
        };
        assert!(error.unauthorized);
        assert!(matches!(
            api.entry().unwrap().get_password(),
            Err(keyring::Error::NoEntry)
        ));
        worker.join().unwrap();
    }

    #[test]
    fn malformed_boards_are_rejected() {
        for (field, value) in [
            ("rows", serde_json::json!(usize::MAX)),
            ("cells", serde_json::json!([])),
            ("status", serde_json::json!("unknown")),
            ("id", serde_json::json!("../me")),
        ] {
            let mut data = view();
            data[field] = value;
            let board: GameView = serde_json::from_value(data).unwrap();
            assert!(board.into_board().is_err());
        }
    }

    #[test]
    fn only_https_or_local_http_origins_are_allowed() {
        for url in [
            "http://wasd.dk",
            "https://user:secret@wasd.dk",
            "https://wasd.dk/path",
            "https://wasd.dk/?token=secret",
        ] {
            assert!(Api::new(url).is_err());
        }
        assert!(Api::new("https://wasd.dk/").is_ok());
        assert!(Api::new("http://127.0.0.1:8000").is_ok());
    }

    #[test]
    fn server_board_conversion_preserves_hidden_cells_without_planting_local_mines() {
        let view: GameView = serde_json::from_value(serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000", "mode": "easy",
            "rows": 9, "columns": 9, "mine_count": 10,
            "cells": vec![serde_json::json!({"mine": false, "revealed": false, "flagged": false, "count": 0}); 81],
            "status": "playing", "elapsed_ms": null, "clock_started": false
        })).unwrap();
        let (_, board, _) = view.into_board().unwrap();
        assert!(!board.started);
        assert!(board.cells.iter().all(|c| !c.mine && !c.revealed));
    }
}
