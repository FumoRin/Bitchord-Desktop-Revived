# Code signing policy

Windows, macOS and Linux releases are built from this repository by [GitHub Actions](.github/workflows/release.yml); nothing is built or signed on a personal machine.

The releases are **not code-signed**. On Windows, SmartScreen may warn about an
unsigned download — choose **More info → Run anyway**. On macOS, right-click the app
and pick **Open** the first time (see the README's installation steps).

## Team

- Committer and reviewer: [LoserDenji](https://github.com/LoserDenji)
- Approvers: [LoserDenji](https://github.com/LoserDenji), [nxfu](https://github.com/nxfu)

## Build-time credentials of the project's own

Two integrations use accounts the project had to register for itself (before 2.0.3 it
rode on ytm-rs's, which belonged to that project's author):

- **Discord** — the application behind Rich Presence. Its ID is baked in at build
  time from the `BITCHORD_DISCORD_CLIENT_ID` variable (Settings → Developer settings
  in the Discord Developer Portal). Without it, the Discord card stays off.
- **Last.fm** — the API key and secret for scrobbling, from the
  `BITCHORD_LASTFM_API_KEY` and `BITCHORD_LASTFM_SECRET` variables (registered at
  [last.fm/api/accounts/create](https://www.last.fm/api/accounts/create)). Without
  them, scrobbling stays off.

Set the three as repository variables before cutting a release; a build without them
ships with both features disabled.

## Privacy

BitChord only talks to the services it needs to play your music:

- **YouTube Music and Google** (`music.youtube.com`, `youtube.com`, `googlevideo.com`, `accounts.google.com`) for browsing, playback, likes, playlists and sign-in. Your sign-in cookie stays on your PC.
- **Music sources you configure** — JioSaavn and Monochrome providers, BitChord addons and any custom provider URL — for the hi-res, lossless and Atmos copies they carry.
- **Lyrics providers** (`lyrics.api.dacubeking.com`, `unison.betterlyrics.org`, `lrc.red`, `lyrics.paxsenix.org`, the LyricsPlus mirrors, `genius.com`) receive the song's title, artist and IDs to find synced lyrics.
- **Apple Music's catalog** (`music.apple.com`, `amp-api.music.apple.com`) is queried for a song's animated cover.
- **The jam relay** (`api.bitchord.kushagrasingh.in`, or the server you set in Settings) relays Listen Together control messages — song IDs and positions, never audio, and only while you're in a room.
- **GitHub** (`github.com`) to check for and download updates, to download `yt-dlp` when an age-restricted song is played on an account without Premium, and to refresh it weekly after that.
- **Discord**, through the Discord app on your PC, when the "Show what I'm playing on Discord" setting is on.
- **Last.fm** when scrobbling is on, and **Spotify** when you use the playlist transfer.
- A custom proxy, if you set one in Settings, carries all of the above.

It sends no analytics or telemetry, and nothing goes to the developer.
