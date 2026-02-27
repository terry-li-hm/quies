# quies

TUI ambient sound mixer — layer noise and nature sounds from your terminal.

```
 quies
 ▸ Brown Noise       ████████████░░░░░░░░  60%
   Pink Noise        ██████░░░░░░░░░░░░░░  30%

 j/k select  h/l volume  m mute  q quit
```

## Install

```sh
cargo install quies
```

## Usage

```sh
quies               # default preset (brown noise)
quies focus         # brown + pink noise
quies deep          # heavy brown, light pink
```

## Controls

| Key | Action |
|-----|--------|
| `j` / `↓` | Select next layer |
| `k` / `↑` | Select previous layer |
| `l` / `→` | Volume up |
| `h` / `←` | Volume down |
| `m` | Mute/unmute layer |
| `q` | Quit |

## Presets

| Name | Layers |
|------|--------|
| `default` | Brown noise at 50% |
| `focus` | Brown 60% + Pink 20% |
| `deep` | Brown 80% + Pink 10% |

## License

MIT
