# Test pictures

Made-up pictures for the tests of pictures LimarePDF places and reads; not in the binaries. Part of LimarePDF
under its own licence.

| File | What | Made with | sha256 |
| --- | --- | --- | --- |
| cmyk-cyan-red.jpg | A 16 × 8 CMYK JPEG, cyan on the left and red on the right, with Adobe's marker (samples stored inverted), as print work writes them | `magick -size 8x8 xc:cyan -size 8x8 xc:red +append -colorspace CMYK -strip -quality 95 cmyk-cyan-red.jpg` (ImageMagick 7.1.2) | `fb0a452131acddd12b9ce98d5da3c5b4ccc357e63617cfb33f33629eaf6ea50a` |
