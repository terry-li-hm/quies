# quies

TUI ambient sound mixer — layer noise and nature sounds from your terminal.

## Install

```sh
cargo install quies
```

## Usage

### Daemon mode (background playback)

```sh
quies start              # start with default preset
quies start focus        # start with focus preset
quies status             # show what's playing
quies vol brown 0.3      # set brown noise to 30%
quies mute pink          # toggle mute on pink noise
quies stop               # stop daemon
```

### TUI mode (interactive)

```sh
quies                    # TUI with default preset
quies focus              # TUI with focus preset
```

```
 quies
 ▸ Brown Noise       ████████████░░░░░░░░  60%
   Pink Noise        ██████░░░░░░░░░░░░░░  30%

 j/k select  h/l volume  m mute  q quit
```

## Presets

| Name | Layers |
|------|--------|
| `default` | Brown noise at 50% |
| `focus` | Brown 60% + Pink 20% |
| `deep` | Brown 80% + Pink 10% |

## License

MIT
