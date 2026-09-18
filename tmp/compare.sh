#!/usr/bin/env bash

if [ "$#" -ne 2 ]; then
    echo "Usage: $0 image1 image2"
    exit 1
fi

img1="$1"
img2="$2"

# Resize larger image to smaller
read w1 h1 <<< "$(magick identify -format '%w %h' "$img1")"
read w2 h2 <<< "$(magick identify -format '%w %h' "$img2")"

if (( w1*h1 > w2*h2 )); then
    magick "$img1" -resize "${w2}x${h2}!" tmp1.png
    cp "$img2" tmp2.png
else
    cp "$img1" tmp1.png
    magick "$img2" -resize "${w1}x${h1}!" tmp2.png
fi

# Convert both to RGB
magick tmp1.png -colorspace RGB tmp1.png
magick tmp2.png -colorspace RGB tmp2.png

# Compute absolute pixel difference
diff=$(magick compare -metric AE tmp1.png tmp2.png null: 2>&1)

# Total number of channels
read w h <<< "$(magick identify -format '%w %h' tmp1.png)"
channels=3
total=$(( w * h * channels ))

# Compute fraction safely in awk (handles scientific notation)
fraction=$(awk -v d="$diff" -v t="$total" 'BEGIN{ f=1-(d/t); if(f<0) f=0; print f }')

echo "Pixel match fraction: $fraction"

rm -f tmp1.png tmp2.png
