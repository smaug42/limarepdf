# Test pictures

Made-up pictures and colour profiles for the tests of pictures LimarePDF places, reads, saves and optimises; not
in the binaries. Part of LimarePDF
under its own licence.

| File | What | Made with | sha256 |
| --- | --- | --- | --- |
| cmyk-cyan-red.jpg | A 16 × 8 CMYK JPEG, cyan on the left and red on the right, with Adobe's marker (samples stored inverted), as print work writes them | `magick -size 8x8 xc:cyan -size 8x8 xc:red +append -colorspace CMYK -strip -quality 95 cmyk-cyan-red.jpg` (ImageMagick 7.1.2) | `fb0a452131acddd12b9ce98d5da3c5b4ccc357e63617cfb33f33629eaf6ea50a` |
| linear-rgb.icc | An RGB colour profile (ICC version 2, display) with sRGB's primaries and straight-line tone curves (gamma 1.0): 128 shows as about 188 in sRGB (Pillow 13's ImageCms: 128 grey → 188, (64, 160, 32) → (137, 208, 99)); its device model isn't "sRGB" | `tools/make_test_icc_profiles.py` (standard library only) | `c07c86e609618f8b099811fcb33cf851011e835df2cb29d4cd7273cc1bcb1a5f` |
| linear-gray.icc | A grey colour profile (ICC version 2, display) with a straight-line tone curve: 128 shows as about 188 (Pillow 13: 128 → 188, 64 → 137) | `tools/make_test_icc_profiles.py` | `b2032b4cf031072fa80fcd064cc82621a7dbcaf5b7d0aff8f492a8727675d139` |
