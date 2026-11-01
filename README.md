# Minesweeper Bevy

A Minesweeper desktop client built with Bevy, with anonymous offline play and
account-bound, server-managed online games.

## Run

```powershell
cargo run
```

Choose Beginner (9x9/10), Intermediate (16x16/40), Advanced (16x30/99),
or Custom. Adjust custom dimensions and mines, then select **New game**.
The first revealed cell is always safe.

- The game header displays `gfx/Makibishi Sweeper.png`, fitted to the available
  width while preserving its aspect ratio.
- Covered, flagged and revealed-mine tiles use the pixel-art images in `gfx/`,
  scaled with nearest-neighbor sampling. Covered tiles select `tile-0.png` through
  `tile-3.png` using a fixed coordinate hash, keeping the pattern stable across
  redraws, zoom changes and new games.
- Left click: reveal; click a revealed number to chord when adjacent flags match.
- Right click: toggle a flag.
- Middle click or left+right together: chord a revealed number when adjacent
  flags match; otherwise flash its uncleared, unflagged neighbors for 200 ms.
- Numbers 1-8 use distinct colors adapted to the dark board.
- **F**: toggle flag mode.
- **N**: new game.
- **F11**: toggle borderless fullscreen; **Escape**: return to windowed mode.
- **F2**: show/hide the online leaderboard without ending the current game.
- **F1**: open/close How to play; **Escape** closes the popup.
- **F5**: toggle exactly 2x board zoom, including numbers and gaps.
  Scroll larger boards with the wheel; **Shift+wheel** scrolls horizontally.
- **Ctrl+Q**: quit. After the first move, unfinished games require confirmation
  (**Enter** to quit, **Escape** to keep playing).
- When a game ends, a popup shows win/loss and why (the mine revealed or a
  misplaced chord flag), the time, whether the score was recorded online, and the
  leaderboard place achieved (or that it would have achieved for unrecorded wins).
  **Escape** dismisses it.
- If the connection fails during a ranked game, the client retries automatically
  with backoff (1, 2, 4, 8, then every 15 s). After a failed move it reloads the
  board first and resends the move only if the server never applied it, so a
  flag is never toggled twice. From the second failure a "Connection problem"
  popup hides the board: **Enter** retries now, **Escape** stops (abandoning the
  ranked game for offline play, or leaving **Submit score** for an unsaved win).
  The ranked clock is server-side and keeps running while disconnected.
- Mines, time, ranking status, flag mode and fullscreen controls share the fixed
  bottom status bar with network messages and errors.

## Account integration

Offline play requires no account and never uploads scores. Select **Sign in**
and enter your existing wasd.dk username and password. Click a field or use
**Tab** to switch; **Enter** submits. Passwords are masked and never saved.
Account creation and password recovery are not part of this client; use the
existing website/account administration.

**Remember login** is opt-in and stores only a player session token in Windows
Credential Manager. Remembered sessions last 30 days; other sessions last
12 hours. On startup the client validates remembered login against the server.
Signing out removes the stored token and revokes the current server session.
Disabled accounts and account session-version changes also invalidate tokens.
Credential-store or network failures are shown explicitly. Secure remembered
login currently supports Windows only.

Signing in or restoring remembered login enables **Online mode** by default
and starts a fresh ranked game (Custom remains offline). Toggle **Online mode**
off if you prefer offline play. Difficulty
selection and **New game** then request boards from the server. All moves and
winning times are server-managed; the client never receives hidden mines.
Completed fixed-mode games submit their score automatically. Failed submissions
can be retried with **Submit score**. After an uncertain move response, use
**Refresh board** before continuing; moves are never silently replayed.
Custom games remain offline and unranked.

**Online leaderboard** shows the shared website's top ten scores for Beginner,
Intermediate and Advanced. Guest entries are marked `*`; account entries use
their current Display Name. Server validation does not prevent automated play.

The default server is `https://wasd.dk`. It needs the new `/api/player` routes
from the corresponding wasd5 server changes deployed before login can work.
For local development:

```powershell
$env:MINESWEEPER_SERVER = 'http://127.0.0.1:8000'
cargo run
```

Only HTTPS origins or loopback HTTP origins are accepted. Redirects are refused
to avoid forwarding credentials to a different endpoint. Requests have bounded
timeouts and run off the UI thread. Saved window preferences are not yet
implemented.

On Windows the client loads Segoe UI from the system Fonts directory for
Unicode account names (including Danish letters); no system font is bundled
or copied. Font-loading errors stop startup with an explicit error.

## Tests

```powershell
cargo test
```
