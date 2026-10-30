# Terminal video experiment

Video encoding, preview, playback and TCP/SSH serving now live in img2irc's library.
`video.rs` is a compatibility entry point for the same code. They need FFmpeg and ffprobe on `PATH` to encode or preview, and an ANSI
terminal to watch.
Audio is omitted.

Build once (OCR is unnecessary for this example):

```sh
cargo build --release --no-default-features --bin img2irc --example video
```

For static x86-64 musl executables, using the same musl compiler toolchain as the
main static build:

```sh
./scripts/build-examples-musl.sh        # all examples
./scripts/build-examples-musl.sh video  # just video
```

Install the Rust target with `rustup target add x86_64-unknown-linux-musl` if
needed. The script requires `x86_64-unknown-linux-musl-gcc`,
`x86_64-unknown-linux-musl-g++`, and `x86_64-unknown-linux-musl-ar` on `PATH`.
It builds release executables in
`target/x86_64-unknown-linux-musl/release/examples/`, with OCR disabled so no
ONNX Runtime build is needed. It can also be invoked from outside the repository.

Encode an MP4, then play the resulting file:

```sh
target/release/examples/video encode clip.mp4 clip.i2v \
  --width 80 --height 24 --fps 24 \
  --palette ansi24 --glyphs half
target/release/examples/video play clip.i2v
```

Encode a short section for testing:

```sh
target/release/examples/video encode clip.mp4 test.i2v --start 00:01:30.5 --duration 5
```

This starts 90.5 seconds into the source and encodes the next 5 seconds.
`--start` (also `--start-time`) and `--duration` accept seconds, `MM:SS`, or
`HH:MM:SS`, including fractional seconds. Start defaults to zero; omitting
duration encodes the remainder of the video. The clip ends sooner if the
source ends. Duration must be greater than zero.

Try more glyphs and processing:

```sh
target/release/examples/video encode clip.mp4 processed.i2v \
  --palette ansi256 --glyphs smooth,half \
  --font-size 20 --line-thickness 1.3 \
  --contrast 20 --quantize-colours 32 --smooth
```

`--palette` selects `ansi24` (24-bit RGB, default) or `ansi256`. `--glyphs`
accepts the usual comma-separated img2irc glyph sets; `half` is a cheap first
test. Your terminal font must support the chosen glyphs. Contrast defaults to
zero and quantization is disabled until requested; its colour limit applies
to each finished frame independently. `--smooth` uses the existing smoothing
with five candidates and one scan order, without neighbor protection or shape
refinement. Encoding can be much slower than playback.

`--line-thickness` accepts fractional values from -10 to 10 (default 0).
Positive values grow dark lines; negative values grow light lines. It runs
before glyph matching and works alongside contrast and quantization.
`--font-size` controls the rendering font height in pixels, from 6 to 256
(default 16), including fractional sizes. It changes glyph-matching resolution;
the terminal's font size is controlled by the terminal itself.

Quantization has two controls: `--quantize-colours 32` caps each frame's finished
art at 32 colours, and `--quantize-threshold 3.5` (also `--quantize-merge`)
merges nearby shades using an OKLab distance threshold from 0 to 100 percent.
The threshold defaults to zero, disabling shade pooling.
Use `--quantize-colours 0 --quantize-merge 3.5` to merge without a colour cap.
Specifying a threshold alone enables a 16-colour limit; omitting both disables
quantization. This runs after glyph matching.

## Tune a live preview

```sh
target/release/examples/video preview clip.mp4 --start 90.5 --duration 5 \
  --glyphs smooth,half --smooth --quantize-colours 32 --quantize-merge 3
```

Preview opens img2irc's existing TUI, with its General, Pipeline, Glyphs and Text
panes. The main executable accepts the same commands:

```sh
target/release/img2irc video preview ~/Videos/dexterslab.mp4 --start 6 --duration 2 --height 60 --glyphs smooth
target/release/img2irc --tui ~/Videos/dexterslab.mp4
```

The file picker (Ctrl+O) accepts videos too. Preview starts paused so you can tune
one frame. Use the existing controls for font, palette, glyphs, smoothing, Repair
Edges and its advanced settings. The Pipeline pane includes Flat Fill with all
region and video-stability controls, line thickness, quantization with its merge threshold,
and img2svg vectorization with tracing, palette and recolouring parameters.

**Blend colours** in General is off by default. This keeps resizing and SVG
rasterization from introducing blended shades, and selects foreground and
background colours separately from actual source pixels under each glyph mask,
including after refinement. `--blend-colours` restores the previous filtering and
colour selection for both preview and encode. FFmpeg also uses RGBA nearest
resizing when this option is off. For exact RGB use `--palette ansi24`; the
256-colour mode still maps those source samples to its fixed palette.

Use **Pipeline → Effects → Darken Outlines** to turn detected dark lines black.
Both preview and encode accept `--darken-outlines`, with `--outline-strength`
(default 100%), `--outline-radius` (3 source pixels), `--outline-threshold`
(40% maximum luminance), and `--outline-contrast` (8% minimum local contrast).
Increase radius for thicker outlines. This targets narrow dark marks rather
than applying a global brightness change; small dark details may also qualify.

- Ctrl+Enter plays/pauses the selected clip; Shift+Enter works too. `[` and `]`
  step by one frame from General, Glyphs, or Video.
- Click or drag the full-width seek bar. Its endpoints select the first and
  last frames; dragging pauses playback. Position accepts clip-relative seconds.
- Tab/Shift+Tab move focus between visible panes on the current page, including
  output and Video; they never change the selected page. Click a page header or
  use Ctrl+Left/Right to change pages. F6 also focuses the video
  controls. Up/Down select a field, Left/Right adjust it,
  and Enter edits it. Ctrl+U clears an input. Escape leaves the controls.
- Page Up/Down seek backward/forward 1% of the selected clip. Shift+Page Up/Down
  use 10%, with a minimum step of one frame.
  Seeking pauses playback and clamps to the selected clip's first/last frame.
- Set Start, Duration, FPS, cache budget, reuse tolerance and temporal reuse
  there. Start is relative to the source; Duration is relative to Start.
- Set Output, then F7 encodes the clip with a snapshot of all editor settings.
  Progress updates after every frame; F7 again cancels. Existing files are
  protected, and cancelled partial encodes are removed.
- Ctrl+Q exits. Escape cancels a preview render.

The last frame's render-stage timings appear below the controls. Processing runs
on a worker; playback slows down when rendering cannot keep up, and obsolete
renders are cancelled when settings change. Output colour edits reuse the base
glyph result without repeating smoothing. Encoded files still work with the
existing player and servers. `img2irc video encode/play/serve/serve-ssh` use the
same backend as the example commands.

`--smooth-shapes` enables the slower shape-refinement pass and implies `--smooth`.
`--glyph-search` (also `--refine-edges`) enables the separate Repair Edges pass.
Both are available in encode and preview. A bounded cache (four frames, up to
32 MiB) reuses exact repeated frames with identical rendering settings, including
the expensive refinement work. It compares the actual decoded pixels; changing
matching settings invalidates reuse. It does not copy smoothed cells between
different images, where neighbouring edges may have changed.

Shape refinement also shares a separate bounded 16 MiB cache of local median-flow
scores between threads and frames. An unchanged patch can reuse that expensive
calculation even when other parts of the frame move. Patch dimensions and actual
pixels must match; final glyph selection and neighbour checks still run normally.
This preserves the refinement result. It does not accelerate every part of
refinement, and the benefit depends on how much local content repeats.

Repair Edges has these advanced controls in both `encode` and the live preview:

| Argument | Default | Meaning |
| --- | --- | --- |
| `--glyph-search-candidates` | 6 | Alternatives per cell (1–64); more increases pair combinations rapidly. |
| `--glyph-search-passes` | 2 | Maximum passes (1–20). |
| `--glyph-search-finalists` | 3 | Locally ranked combinations checked against the full image; 0 checks all. |
| `--disc-threshold` | 0.5 | Minimum defect strength; 0 searches every non-flat cell. |
| `--glyph-search-cache-mb` | 64 | Local ranking cache budget in MiB; 0 disables it. |
| `--glyph-search-reuse-tolerance` | 0 | Allowed RGB-channel drift (0–32) when reusing rankings; 0 requires exact pixels. |
| `--glyph-search-temporal` | off | Carry repaired glyphs through stable regions and search changed regions plus a two-cell margin. |

The local cache compares source patches, surrounding rendered pixels, glyph
shapes, colours and search configuration. A glyph and an inverse glyph with
swapped foreground/background colours share a canonical key; cache hits map
back to the current glyph choices and preserve their tie ordering. An unchanged patch can reuse its
ranked finalists when other parts of the frame move. Full-image acceptance is
always recalculated. Compression noise often prevents exact hits; a small
`--glyph-search-reuse-tolerance`, such as 4, permits approximate local reuse.
This can change which finalists are considered and the resulting glyphs, but
every accepted change must still improve the current frame's perceptual score.
It does not skip decoding, initial matching, or the full-image checks. Memory
and speed benefits depend on the footage. `--profile` prints per-frame stage
times and cumulative local-cache hits during encoding.

For video, `--glyph-search-temporal --glyph-search-reuse-tolerance 8` also carries
the previous frame's repaired glyphs into stable regions, allowing small RGB
changes from compression. Motion invalidates a two-cell margin, and changing the
render configuration invalidates the history. The complete warm start must beat
or tie the current frame's fresh initial fit; otherwise it falls back to a full
search. It still verifies the final whole-image score. This mode can choose
different glyphs and miss improvements in unchanged areas, so it is opt-in.
History retains one frame, capped at 64 MiB in addition to the ranking cache.
`--profile` reports the number of cells carried forward as `temporal-cells`.

For a less expensive search, try `--glyph-search-candidates 3
--glyph-search-passes 1 --glyph-search-finalists 1`. These change the amount of
search, so compare the picture in preview before encoding a whole clip.

`--img2svg` is another source-image effect, using the img2svg library's region
quantization and vector tracing before glyph fitting. All settings live in the
shared Pipeline editor, including scrollable controls for its full parameter
list. Both `encode` and `preview` accept the same `--img2svg-*` conversion flags:

```sh
target/release/examples/video preview ~/Videos/dexterslab.mp4 \
  --start 6 --duration 2 --height 60 --glyphs smooth \
  --img2svg --img2svg-preset logo --img2svg-colors 16 \
  --img2svg-recolor-mode mode
```

`--img2svg-recolor-mode mode` uses the most common original colour instead of
averaging a colour bucket. `--img2svg-max-size` bounds tracing resolution
without changing output geometry. `--img2svg-colors 0` selects automatic palette
sizing. See the [complete parameter reference](../docs/usage.md). Conversion is
in memory, with no img2svg executable or temporary SVG files required. The
effect treats frames independently.

`--flat-fill` is an experimental automatic preprocessing effect for flat-shaded
animation. It grows connected regions within a small perceptual colour range,
rejects gradual ramps and small details, then fills each suitable region with
an observed colour nearest its perceptual mean. Strong outlines and alpha
boundaries separate regions. It needs no palette size or merge setting, and
runs before glyph matching; output quantization remains optional. Its controls
are available in both `encode` and live preview:

| Argument | Default | Meaning |
| --- | --- | --- |
| `--flat-fill-tolerance` | 2.5 | Maximum OKLab distance from a region's seed, in percent. Higher joins more shades. |
| `--flat-fill-flatness` | 1.2 | Maximum within-region colour standard deviation, in percent. Higher accepts more texture. |
| `--flat-fill-min-area` | 16 | Minimum region size in raster pixels, independently of terminal cells. |
| `--flat-fill-gradient` | 0.6 | Threshold for gradient protection, in percent. Zero disables protection. |
| `--flat-fill-strength` | 100 | Blend toward the chosen colour, in percent. Zero leaves pixels unchanged. |
| `--flat-fill-temporal-tolerance` | 1 | Video colour stability in OKLab percent. Zero disables frame history. |

During sequential preview/encoding, Flat Fill matches current regions to the
previous frame by colour and at least 70% pixel overlap. It retains a previous
fill colour only within the stability tolerance, and paints the current region's
pixels. Seeks and rendering-setting changes reset history. History is bounded
to 128 MiB. Independent `encode --workers` requires
`--flat-fill-temporal-tolerance 0` when Flat Fill is enabled; its default history
depends on consecutive frames.

For example, add `--flat-fill-tolerance 5 --flat-fill-flatness 2
--flat-fill-strength 75` for broader shade grouping with partial blending.
`--flat-fill` still enables the effect; changing its controls while it is off
does not enable it. Enter accepts exact values in the preview, including integer
pixel counts for minimum area. The shared editor's encode action retains these controls.
This is a
conservative heuristic, not semantic object recognition or a tracked palette
across frames. `--region-guidance` additionally enables the existing connected
region guidance for glyph colours and boundaries. Both toggles are live controls.

```sh
target/release/examples/video preview ~/Videos/dexterslab.mp4 \
  --start 5 --duration 2 --glyphs smooth,half --flat-fill \
  --glyph-search --glyph-search-candidates 3 --glyph-search-passes 1 \
  --glyph-search-finalists 1
```

On a six-frame section of `dexterslab.mp4` starting at 5 seconds, at 40 columns,
12 fps and one search pass, reducing candidates from 6 to 3 and finalists from 3
to 1 reduced encode time from about 34 to 17 seconds. This trades search
thoroughness for speed. Exact patch caching roughly halved time on a separate
mostly-static synthetic clip with a moving object, with byte-identical output;
it had no hits on the compressed Dexter section. Temporal mode also gave little
benefit on that section. These are workload-dependent results, not a general
speedup claim.

Native Repair Edges now prepares independent neighbour-pair rankings in parallel,
using incremental perceptual feature caches within each patch. A version check
detects intervening glyph commits; affected rankings are refreshed against the
current neighbourhood before use. Commits retain their original serial order,
and close local ranking decisions use the original raster-order scorer.
Full-image feature updates combine pending restores with nearby edits.
This accelerates uncached frames without reducing candidates, passes, finalists,
resolution or scoring criteria. Encoding reports progress after every frame,
including the elapsed render time. `--profile` reports search stages; add
`IMG2IRC_REPAIR_PROFILE=1` to inspect their internal breakdown.
The detailed profile includes batched jobs and local precision rechecks, and
separates patch preparation, copying, painting/features and perceptual scoring.
Worker durations overlap; their sum is not elapsed frame time. Add
`IMG2IRC_REPAIR_SCORE_PROFILE=1` for the much heavier per-cache scorer diagnostics.
The editor's "final score" timing is only the final reporting pass; scores used
inside Repair Edges are included in its Repair Edges timing.

For independent frames, `encode --workers 2` renders two frames concurrently
and writes them in source order. It bounds decoded and completed frames to one
worker batch and splits the local ranking cache budget between workers. This
requires temporal search off and `--glyph-search-reuse-tolerance 0`; these modes
depend on earlier frames. The default is one worker, and preview keeps one
frame in flight so changes can cancel obsolete work.

The native benchmark at the user's settings (`--start 6 --glyphs smooth
--flat-fill --height 60 --line-thickness 1 --glyph-search-candidates 3
--glyph-search-passes 1 --glyph-search-finalists 1`, with 1024 MiB cache,
tolerance 16 and temporal mode) rendered two consecutive frames in 8.1 seconds,
versus 19.1 seconds for the reference, with byte-identical output. In a separate
independent-frame comparison, `--workers 2` took 4.9 seconds versus 7.8 seconds
for one worker, also with identical output. Both tests used native release
binaries, not musl. Frame concurrency improves throughput; each simultaneous
frame can take longer because it shares CPU resources.

The old `vector-gpu-next` branch accelerates independent initial glyph fitting,
using a different RGB fitting rule. It does not implement the current perceptual
Repair Edges scorer. Initial matching was a tiny fraction of encode time in the
tested animation clips, so that backend has not been restored. A useful GPU
implementation would keep resources alive across frames and batch independent
local candidate scores; accepted pair changes still depend on earlier commits.

Profiling two Dexter frames from 6s with the settings above attributed about
43% of measured ranking worker durations to scoring, 37% to patch preparation,
and 20% to copying and painting/features. Scoring is the largest individual
category, but preparation is close. These are overlapping worker durations,
not wall-clock percentages. Initial glyph matching took 4–6ms; full-image
checks/restoration took about 1.5s per frame. A GPU search needs to address
preparation and candidate scoring together; restoring only the old initial
matcher does not address those costs. There is currently no GPU Repair Edges
scorer in this branch.

The default output bounds are 80 columns by 24 rows at 24 fps. Width and height
are terminal cells, not source pixels: a half-block still occupies one cell.
The encoder preserves the video aspect ratio using the font's cell proportions,
handles rotation and nonsquare source pixels, and writes the actual fitted
dimensions without adding black padding. Q, Escape, and Ctrl-C exit and restore
the terminal. Existing output files are never overwritten.

The local player anchors the image at the top left and fits oversized frames
within the terminal, including after a resize. Existing files with large padded
canvases also work. Fitting samples the encoded glyph cells without rerunning
image processing; it preserves the full frame rather than cropping an edge.
To keep the original cell size instead:

```sh
target/release/examples/video play clip.i2v --ignore-terminal-size
```

With this override, anything outside the terminal is clipped. It never scrolls
the image to make room for off-screen rows.

The player buffers four frames before starting and reads ahead on a background
thread. After the first frame, playback, preview and both servers send only
changed terminal cells, using ANSI cursor positions and colour changes. Dense
updates use a complete redraw if it is smaller. Resizing forces a redraw, and
skipped source frames are compared to the last frame actually sent. Identical
frames send no picture bytes. Initial frames position every row explicitly, without
newlines that could scroll the image at the bottom edge. It uses synchronized
updates where supported and schedules against one monotonic clock. Late
frames are skipped so playback does not accumulate delay. Smoothness still
depends on terminal throughput; reduce dimensions, fps, or colours if needed.

The `.i2v` file stores independent complete frames; delta updates are generated
by the player and servers. Output uses absolute cursor jumps between changed
runs and reuses colour settings within each update. It does not yet choose the
shortest relative cursor sequences. In a local playback sample of the first
48 frames at 200×76 cells, Dexter's truecolour stream used 3.82 MB versus
5.79 MB for full redraws (34% less); the 256-colour version used 2.23 MB versus
3.30 MB (32% less). Neither sample dropped frames; savings depend on the scene.

## Stream to netcat clients

Copy the built `video` executable and an encoded `.i2v` file to your VPS, then run:

```sh
./video serve clip.i2v --bind 0.0.0.0:2323
```

Allow incoming TCP connections on that port. Viewers connect from an ANSI terminal:

```sh
nc your-vps.example 2323
```

Each connection starts the video from the beginning and closes automatically
after its final frame. Multiple viewers play independently; the server keeps
accepting new connections. Stop the server with Ctrl-C. No FFmpeg or terminal
is needed on the server to play an already encoded file.

The server sends plain ANSI over TCP, restores the viewer's screen and cursor
before closing, and skips late frames using the same timing policy as the local
player. It positions rows at the top left without scrolling. Plain netcat does
not report terminal dimensions. To fit the whole video for viewers with a known
terminal size, set it on the server:

```sh
./video serve clip.i2v --bind 0.0.0.0:2323 --terminal-size 286x70
```

This size applies to every connection. Without it, viewers need a terminal large
enough for the encoded dimensions. File reading is streamed per connection, so
the entire video is not loaded into memory. Socket writes time out after five
seconds if a client stops receiving. `--bind` defaults to `0.0.0.0:2323`.

## Public SSH playback

The same executable can act as a dedicated SSH server. Anyone can connect using
any username, without a password or a system account, watch once, then disconnect:

```sh
./video serve-ssh clip.i2v --bind 2222
ssh -p 2222 slm.gangnet.org
```

`clip.i2v` is the fallback video. Each SSH username selects `<username>.i2v`
from the server's working directory when that file is readable and has a valid
video header. For example, put `dexter.i2v` in the directory you run the
server from, then connect with:

```sh
ssh -p 2222 dexter@slm.gangnet.org
```

This plays `./dexter.i2v`; missing/unreadable selections fall back to `clip.i2v`.
Names are matched exactly, including case, and path separators are rejected.
The fallback path can point elsewhere; username selections always use the
server's working directory. Files are opened per playback, so adding a named
video does not require restarting the server. A bare `ssh HOST` uses your local
username and follows the same selection rule.

For the exact command `ssh slm.gangnet.org`, listen on port 22:

```sh
./video serve-ssh clip.i2v --bind 22
```

Port 22 needs permission to bind privileged ports and must be available on that
IP. Keep administrative SSH on its existing port (for example 22022), or another
IP. The video server does not use or change the system SSH configuration.
The default SSH listener is `[::]:2222`, accepting both IPv4 and IPv6. This also
applies to SSH's port-only shorthand, so hosts with both A and AAAA DNS records
work. Use `--bind 0.0.0.0:2222` for IPv4 alone. `--bind` accepts a port or an
`IP:PORT` address in both serving modes.

The first run creates `video_ssh_host_ed25519` with mode 0600 on Unix. Keep that
file across restarts so returning viewers see the same host identity. Set
`--host-key /path/to/key` to use another location or an existing unencrypted
OpenSSH key. Viewers normally accept the host fingerprint on their first visit.

SSH reports terminal dimensions and window changes, so there is no
`--terminal-size` setting. Frames fit at the top left, including after a resize.
When a video must shrink to fit, space is reserved at the terminal's bottom for
`INCREASE WINDOW SIZE FOR BEST VIEWING EXPERIENCE`. It wraps in narrow windows
and disappears when the original video fits. This notice also applies to local
playback and TCP serving with `--terminal-size`; it does not appear for deliberate
clipping with `--ignore-terminal-size`.
Q, Escape or Ctrl-C exits early; the terminal is restored on exit. Each viewer
gets independent playback with four-frame read-ahead, a fixed playback clock,
late-frame skipping and backpressure from SSH flow control. No FFmpeg is needed
to serve an encoded file.

The service only plays the selected or fallback video. Remote commands, SFTP, forwarding,
and additional concurrent playback channels are rejected. Sequential plays can
reuse a connection when the client enables SSH connection sharing. Connections
without a PTY use 80x24 bounds. Network compression is supported when requested
by the client:

```sh
ssh -C slm.gangnet.org
```

Bare `ssh HOST` works without compression; OpenSSH does not enable it by default.
The native and musl example build commands above include SSH support. Its Rust
dependencies are scoped to examples/tests, leaving ordinary application builds
without an SSH server dependency.

The experimental `.i2v` format is uncompressed UTF-8 JSON Lines: a header
(`format`, `version`, `width`, `height`, `fps`), one JSON string containing a
complete ANSI frame per line, then `null` as an end marker. Each frame includes
cursor positioning and colour resets, so skipped frames need no reconstruction.
Files can be large; start with a short clip. Playback memory is bounded by the
frame buffers, rather than video length. There is no audio, seeking, resizing
of encoded frames, or stable-format promise in this proof of concept.
