# img2irc (1.1.1)

![img2irc braille example](https://i.imgur.com/ZEJwuOb.png)
![img2irc preview](https://i.imgur.com/0omljq5.png)

*img2irc* is a premiere command-line utility which converts images to irc/ansi art, with a lot of post-processing filters

# how to install

- ### download the linux binary

  statically linked with musl, works on all x86_64 linux platforms

      cd /tmp
      wget https://github.com/waveplate/img2irc/releases/download/v1.1.0/img2irc-1.1.0-linux-x86_64.tar.gz
      sudo tar -xzf img2irc-1.1.0-linux-x86_64.tar.gz -C /usr/local/bin --strip-components=1 img2irc-1.1.0/img2irc
      rm -rf img2irc-1.1.0-linux-x86_64.tar.gz


- ### install with `yay` (arch linux)
  
      yay -S img2irc

> [!NOTE]
> if you like this project, i would appreciate you giving it a vote on the [aur](https://aur.archlinux.org/packages/img2irc)!

- ### install with `cargo`
  
      cargo install img2irc-rs

  the binary will be installed to `~/.cargo/bin/img2irc`

# usage

`img2irc <URL or PATH> [OPTIONS]`

| option                                 | description                                                   | default value |
|----------------------------------------|---------------------------------------------------------------|---------------|
| image                                  | image url or file path                                        | required      |
| -w, --width                            | output image width in columns                                 | auto          |
| -H, --height                           | output image height in rows                                   | auto          |
| --scale                                | scaling factors (x:y, e.g., "2:2")                            | none          |
| --aspect                               | final aspect ratio (x:y, e.g., "2:1")                         | none          |
| --crop                                 | crop image ("x1,y1,x2,y2")                                    | none          |
| --filter                               | sampling filter                                               | nearest       |
| --rotate                               | rotate degrees                                                | 0             |
| --fliph                                | flip horizontal                                               | false         |
| --flipv                                | flip vertical                                                 | false         |

### colours rendering options (select one)

`irc` mode has 99 colours, (6.62-bit)

`ansi` mode has 256 colours (8-bit)

`ansi24` has 16777216 colours (24-bit)

| option                                 | description                                                   | 
|----------------------------------------|---------------------------------------------------------------|
| --irc                                  | use irc99 colours                                             |
| --ansi                                 | use 8-bit ansi colours                                        |
| --ansi24                               | use 24-bit ansi colours                                       |

### pixel rendering options (select one)

`halfblock` mode increases the vertical resolution, doubling the total resolution for a given size

`quarterblock` mode increases both the vertical and horizontal resolution by twofold, quadrupling the total resolution for a given size

`braille` mode uses 2x4 dot patterns to represent pixels, increasing resolution eightfold

| option                                 | description                                                   |
|----------------------------------------|---------------------------------------------------------------|
| --braille                              | use braille pixels                                            | 
| --hb, --halfblock                      | use halfblock pixels                                          | 
| --qb, --quarterblock                   | use quarterblocks pixels                                      |

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
| --nograyscale                          | exclude grayscale colours from the palette                    | false         |
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

![img2irc braille example](https://i.imgur.com/MxroWUb.png)
