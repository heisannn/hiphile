# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0](https://github.com/heisannn/hiphile/compare/v0.3.0...v0.4.0) - 2026-11-08

### Added

- *(player)* add a mode that skips instrumental songs
- *(audio)* trim silence and crossfade between songs on PCM
- *(audio)* resample with a clean-room soxr
- *(audio)* resample with a port of r8brain-free-src
- *(audio)* fade the sound on play, pause and stop

### Fixed

- *(audio)* keep a written fade to go back to, and hear a seek before fading

## [0.3.0](https://github.com/heisannn/hiphile/compare/v0.2.0...v0.3.0) - 2026-11-07

### Added

- *(player)* add a mini player window
- *(updater)* install new versions from GitHub releases
- *(app)* give the app an icon on macOS and Windows
- *(audio)* run the device at a multiple of each song's rate
- *(audio)* add an exclusive mode for PCM
- *(audio)* play as DSD over DoP
- *(audio)* choose the resampling quality in settings
- *(player)* add a favorite star for the current song

### Fixed

- *(app)* quit when the main window closes

### Other

- keep only the comments the code cannot say itself

## [0.2.0](https://github.com/heisannn/hiphile/compare/v0.1.0...v0.2.0) - 2026-11-05

### Added

- *(favorites)* add favorite songs, albums and artists
- *(player)* move lyrics and queue beside the volume, Settings to the title bar
- *(settings)* turn the Discord status off
- *(search)* notify when the dictionaries can't be had
- *(search)* find kanji by romaji across the library
- *(audio)* offer 705.6 and 768 kHz sample rates
- *(home)* reload the recommendations
- *(player)* answer to the system's media controls

### Fixed

- *(covers)* ask for covers close to the size they show
- *(audio)* keep playback whole and controls live on a slow connection

## [0.1.0](https://github.com/heisannn/hiphile/releases/tag/v0.1.0) - 2026-11-04

### Added

- *(lyrics)* show synced lyrics beside a large cover
- *(title-bar)* show the app's name instead of the server's
- *(player)* repeat one or all songs, and shuffle
- *(navigation)* go back and forward between screens and selections
- *(artists)* add Artists and Songs, and call Library Albums
- *(settings)* set a limit on the cover cache, 8 GiB unless told otherwise
- *(playlists)* browse, play and edit the server's playlists
- *(presence)* show the song playing as the Discord status
- *(home)* add a shelf of the most played albums
- *(settings)* switch between light and dark
- *(home)* recommend albums, picked anew each day
- *(home)* show the songs played last and the albums added last
- *(home)* add an empty home screen and switch to it from the title bar
- *(settings)* set the song cache's limit, see its size and clear it
- *(settings)* choose the output device and fix the sample rate
- *(queue)* show the play queue beside the library and edit it
- *(catalog)* cache the library locally and sync it in the background
- seek, volume and now-playing marker; seek downloads by range
- *(config)* keep settings in ~/.config/hiphile and passwords in the keychain
- add OpenSubsonic client core, CLI and minimal GUI

### Fixed

- *(windows)* keep release builds from opening a console window

### Other

- license under MIT
- *(ui)* bind the keys left to a list's buttons on their own
- wip
- initialize project
