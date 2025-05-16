### img2irc (1.3.0)

*img2irc* is a premiere command-line utility which converts images to irc/ansi art, with a lot of post-processing filters

`$ img2irc https://i.imgur.com/B9syzEm.png --render ansi --blocks --width 132 --contrast 50 --nograyscale`

> &nbsp;
>![img2irc block example](https://i.imgur.com/B9syzEm.png)
> &nbsp; 

# how to install

- ### one-line installer (recommended)

  installs `img2irc` to `/usr/local/bin`

      curl -sL https://github.com/waveplate/img2irc/releases/download/v1.3.0/img2irc-1.3.0-linux-x86_64.tar.gz | sudo tar -xzf - --strip-components=1 -C /usr/local/bin img2irc-1.3.0/img2irc

  statically linked with musl, works on all x86_64 linux platforms

- ### install with `yay` (arch linux)
  
      yay -S img2irc-bin

  or

      yay -S img2irc

  if you prefer to compile it yourself

> [!NOTE]
> if you like this project, i would appreciate you giving it a vote on the [aur](https://aur.archlinux.org/packages/img2irc)!

# usage

`img2irc <URL or PATH> [OPTIONS]`

| option                                 | description                                                   | default value |
|----------------------------------------|---------------------------------------------------------------|---------------|
| image                                  | image url or file path                                        | required      |
| -w, --width                            | output image width in columns                                 | auto          |
| -H, --height                           | output image height in rows                                   | auto          |
| --scale                                | scaling factors (x:y, e.g., "2:2")                            | none          |
| --crop                                 | crop image ("x1,y1,x2,y2")                                    | none          |
| --filter                               | sampling filter                                               | nearest       |
| --rotate                               | rotate degrees                                                | 0             |
| --fliph                                | flip horizontal                                               | false         |
| --flipv                                | flip vertical                                                 | false         |

### colours rendering modes

| option                                 | description                                                   | 
|----------------------------------------|---------------------------------------------------------------|
| --render                               | colour rendering mode (default: `irc`)                        |

`irc` mode has 99 colours, (6.62-bit)

`ansi` mode has 256 colours (8-bit)

`ansi24` has 16777216 colours (24-bit)

### pixel rendering modes (select one)

| option        | description                   |
|---------------|-------------------------------|
| `--braille`   | use braille pixels            |
| `--blocks[=types]` | use block pixels of the provided types. defaults to `full,half,quarter,eighth,triangle,corner,geometric,box,legacy` |

#### braille mode

`--braille` uses 2×4 braille dot patterns, doubling horizontal and octupling vertical resolution

#### block modes

| type      | description          | unicode range  |
|-----------|----------------------|----------------|
| `full`      | full block           | 0x2588         |
| `half`      | half block           | 0x2580-0x2590  |
| `quarter`   | quarter block        | 0x2596-0x259F  |
| `eighth`    | eighth block         | 0x2581-0x2595  |
| `triangle`  | triangle shapes      | 0x25B2-0x25C0  |
| `corner`    | corner triangles     | 0x25E2-0x25E5  |
| `geometric` | geometric patterns   | 0x25A0-0x25FF  |
| `box`       | box-drawing glyphs   | 0x2500-0x257F  |
| `legacy`    | legacy block styles  | 0x1FB00-0x1FBFF|

specifying `--blocks` with no value uses all available glyph types

`full` only uses the fullblock glyph

`half` increases the vertical resolution twofold

`quarter` increases both vertical and horizontal resolution twofold

`eighth` increases resolution in one axis at a time up to eightfold

`triangle` uses triangle shapes

`corner` uses corner triangle shapes

`geometric` uses some geometric shapes

`box` uses some box-drawing characters

> see `src/chars.rs` for the actual glyphs and bitmaps or read `utils/README.md` for information on how to generate a custom `chars.rs` file

### image processing options

| option                                 | description                                                   | default value |
|----------------------------------------|---------------------------------------------------------------|---------------|
| -b, --brightness                       | adjust brightness (0 = no change)                             | 0.0           |
| -c, --contrast                         | adjust contrast (0 = no change)                               | 0.0           |
| -g, --gamma                            | adjust gamma (0 to 255)                                       | 0.0           |
| -s, --saturation                       | adjust saturation (0 = no change)                             | 0.0           |
| -u, --hue                              | rotate hue (0 to 360)                                         | 0.0           |
| -i, --invert                           | colors are inverted, opposite on the color wheel              | false         |
| -d, --dither                           | dithering (1 to 8)                                            | 0             |
| -B, --luma-brightness                  | adjust luma brightness (braille only)                         | 0.0           |
| -C, --luma-contrast                    | adjust luma contrast (braille only)                           | 0.0           |
| -G, --luma-gamma                       | adjust luma gamma (braille only)                              | 0.0           |
| -S, --luma-saturation                  | adjust luma saturation (braille only)                         | 0.0           |
| -I, --luma-invert                      | luminance is inverted (braille only)                          | false         |
| --colorspace                           | colourspace (hsl, hsv, hsluv, lch)                            | hsv           |
| --grayscale                            | converts image to black and white                             | false         |
| --nograyscale                          | exclude grayscale when picking nearest colour in palette (unless already grayscale)                  | false         |
| --pixelize                             | pixelize pixel size                                           | 0             |
| --boxblur                              | simple average of all the neighboring pixels surrounding one  | false         |
| --gaussianblur                         | gaussian blur radius                                          | 0             |
| --oil                                  | oil filter ("[radius],[intensity]")                           | none          |
| --halftone                             | made up of small dots creating a continuous-tone illusion     | false         |
| --sepia                                | brownish, aged appearance like old photographs                | false         |
| --normalize                            | adjusts brightness and contrast for better image quality      | false         |
| --noise                                | random variations in brightness and color like film grain     | false         |
| --emboss                               | gives a raised, 3d appearance                                 | false         |
| --identity                             | no modifications, unchanged image                             | false         |
| --laplace                              | enhances edges and boundaries in an image                     | false         |
| --denoise                              | reduces noise for a cleaner, clearer image                    | false         |
| --sharpen                              | increases clarity and definition, making edges and details more distinct | false         |
| --cali                                 | cool blue tone with increased contrast                        | false         |
| --dramatic                             | high contrast and vivid colors for a dramatic effect          | false         |
| --firenze                              | warm, earthy tones reminiscent of tuscan landscapes           | false         |
| --golden                               | warm, golden glow like sunset light                           | false         |
| --lix                                  | high-contrast black and white appearance with increased sharpness | false         |
| --lofi                                 | low-fidelity, retro appearance like old photographs or film       | false         |
| --neue                                 | clean, modern appearance with neutral colors and simple design    | false         |
| --obsidian                             | dark, monochromatic appearance with black and gray shades         | false         |
| --pastelpink                           | soft, delicate pink tint like pastel colors                       | false         |
| --ryo                                  | bright, high-contrast appearance with vivid colors and sharp details | false         |
| --frostedglass                         | blurred, frosted appearance as if viewed through semi-transparent surface | false         |
| --solarize                             | strange, otherworldly appearance with inverted colors and surreal atmosphere | false         |
| --edgedetection                        | highlights edges and boundaries in an image                     | false         |
