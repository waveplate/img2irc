#!/bin/bash
echo OLD
time /home/malcom/dev/img2irc-good/target/release/img2irc $1 --render irc  --font-size 14  --contrast 70 --width 90 --blocks polygons,eighth,space --connected | llb
echo ""
echo NEW
time i2i $1 --font-size 14  --contrast 70 --width 90 --blocks polygons,eighth,space --smooth --render irc | llb
