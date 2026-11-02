# Minesweeper Bevy

A Minesweeper desktop client built with Bevy, with anonymous offline play and
account-bound, server-managed online games.

## Run

```powershell
cargo run
```

The startup sound plays `sfx/makibishi_intro.ogg` once and removes its audio
entity when playback finishes. Bevy audio and Ogg Vorbis support are enabled.

Windows builds embed `gfx/makibishi-icon.png` as the executable icon and use it
for the running window too. The icon is compiled in; no external icon file is
needed at runtime. Building on Windows requires the Windows SDK resource compiler.

Choose Beginner (9x9/10), Intermediate (16x16/40), Advanced (16x30/99),
or Custom in the controls below the board. **Custom** opens a modal: adjust rows,
columns and makibishi, then select **Start** (or press **Enter**). **Reset** restores
10x10 with 15 makibishi; **X** or **Escape** closes without changing the current game.
The last started custom game's settings persist across app restarts in
`%LOCALAPPDATA%\minesweeper-bevy\custom.json` on Windows (the user configuration
directory on other platforms). **New game** repeats the current game type.
The first revealed cell is always safe.

- Step on five makibishi across games within any 60-second window and Ruru slides in
  from the left above the menu over three seconds, stops at screen center, then
  fades away over two seconds. Each loss counts once (including bad chords),
  online or offline. The count resets after triggering and on app restart.
  Ruru uses `gfx/ruru.png` at 4x size with nearest-neighbor sampling.
  While Ruru is visible, the status bar says "ルル doesn't judge.." before
  returning to the normal game status.
- The game header displays `gfx/Makibishi-Title.png`, fitted to the available
  width while preserving its aspect ratio.
- Covered, flagged and revealed-makibishi tiles use the pixel-art images in `gfx/`,
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
- **F12**: toggle borderless fullscreen; **Escape**: return to windowed mode.
- **F2**: show/hide the online leaderboard without ending the current game.
- **F1**: open/close How to play; **Escape** closes the popup.
- **F5**: toggle exactly 2x board zoom, including numbers and gaps.
  Scroll larger boards with the wheel; **Shift+wheel** scrolls horizontally.
  Edges with off-screen tiles show an 8-pixel inward fade, reaching 50% black at
  the visible edge. The indicators disappear when that side is fully in view.
  Centered chevrons point toward the hidden tiles just outside each fade, hiding
  when the pointer is within 24 pixels of their center.
- **Ctrl+Q**: quit. After the first move, unfinished games require confirmation
  (**Enter** to quit, **Escape** to keep playing).
- Losing displays `gfx/MonkOuch.png` centered over a darkened background instead
  of a modal. Ruru remains undimmed when visible. Any mouse click or **Escape**
  dismisses the picture without activating the controls behind it.
  The triggered makibishi stays outlined in yellow on the board until a new
  game starts, including when a misplaced chord flag caused the loss.
- Winning shows `gfx/MonkAtRest.png` on the left over a darkened background, with
  the current mode's top ten on the right. A chevron marks the just-recorded score;
  if absent, their time appears below the board. Unrecorded games always show
  their time, with an unranked note for offline games.
  Custom wins show time and the unranked note without a
  leaderboard. The recording-status line includes the finished time.
  Loading, submission and leaderboard errors remain visible.
  Any mouse click or **Escape** dismisses the results; Ruru stays undimmed.
- If the connection fails during a ranked game, the client retries automatically
  with backoff (1, 2, 4, 8, then every 15 s). After a failed move it reloads the
  board first and resends the move only if the server never applied it, so a
  flag is never toggled twice. From the second failure a "Connection problem"
  popup hides the board: **Enter** retries now, **Escape** stops (abandoning the
  ranked game for offline play, or leaving **Submit score** for an unsaved win).
  The ranked clock is server-side and keeps running while disconnected.
- Difficulty, **New game**, makibishi and time sit below the board. The black footer
  has clickable account/mode and leaderboard controls on the left, status and
  error messages in the center, and flag/help/zoom/fullscreen controls on the
  right. Offline indicators are red, including the status beneath the banner.

## Account integration

Offline play requires no account and never uploads scores. Select **Not signed in**
to open the centered sign-in modal over the current board and enter your existing
wasd.dk username and password. Click a field or use **Tab** to switch; **Enter**
submits. The Sign in button is enabled once both fields are filled.
**Cancel**, **X**, or **Escape** closes the modal and clears the password without
changing the current game. Passwords are masked and never saved.
Account creation and password recovery are not part of this client; use the
existing website/account administration.

**Remember login** is opt-in and stores only a player session token in Windows
Credential Manager. Remembered sessions last 30 days; other sessions last
12 hours. On startup the client validates remembered login against the server.
Clicking **Signed in (name)** signs out, removing the stored token and revoking
the current server session.
Disabled accounts and account session-version changes also invalidate tokens.
Credential-store or network failures are shown explicitly. Secure remembered
login currently supports Windows only.

Signing in or restoring remembered login enables **Online mode** by default
and starts a fresh ranked game (Custom remains offline). Click the footer's mode
label to toggle online/offline mode. Difficulty
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
or copied. Ruru's status message uses the system Yu Gothic font for its Japanese
characters; other status text keeps Segoe UI. Font-loading errors stop startup
with an explicit error.

## Tests

```powershell
cargo test
```
