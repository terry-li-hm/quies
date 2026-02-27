# quies

TUI ambient sound mixer — layer lofi, noise, and nature sounds from your terminal.

## Install

```sh
cargo install quies
```

Requires [yt-dlp](https://github.com/yt-dlp/yt-dlp) for YouTube audio:
```sh
brew install yt-dlp
```

## Usage

### Daemon mode (background playback)

```sh
quies start              # start with default preset
quies start focus        # start with focus preset
quies status             # show what's playing
quies vol brown 0.3      # set brown noise to 30%
quies mute pink          # toggle mute on pink noise
quies add lofi https://youtu.be/0NCrui_dYJY  # add YouTube audio layer
quies add rain https://example.com/rain.mp3   # add direct audio URL
quies stop               # stop daemon
```

### TUI mode (interactive)

```sh
quies                    # TUI with default preset
quies focus              # TUI with focus preset
```

```
 quies
 ▸ ~ Brown Noise     ████████████░░░░░░░░  60%
   ~ Pink Noise      ██████░░░░░░░░░░░░░░  30%
   ♪ lofi            ██████████░░░░░░░░░░  50%

 j/k select  h/l volume  m mute  q quit
```

### Audio layers

Add audio from YouTube or direct URLs to mix with noise generators:

```sh
quies start
quies add ambient https://youtu.be/0NCrui_dYJY   # downloads via yt-dlp
quies add rain https://example.com/rain.mp3       # downloads via curl
quies vol ambient 0.3                             # adjust volume
quies status                                      # shows download progress
```

- YouTube URLs use yt-dlp (must be installed)
- Direct HTTP URLs use curl
- Files are cached — re-adding the same URL is instant
- Audio loops seamlessly
- Max file size: 200MB

## Presets

| Name | Layers |
|------|--------|
| `default` | Brown noise at 50% |
| `focus` | Brown 60% + Pink 20% |
| `deep` | Brown 80% + Pink 10% |

## License

MIT
