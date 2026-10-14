# Quick start

## The desktop app

Open a file with the Open button, Ctrl+O, or by dropping it on the window, or from a terminal:

```
mpe-gui report.pdf --page 3       # open at page 3
mpe-gui scan.tif photo.jpg        # images open as a new document, one page per image
```

Click what you want to work on, and the panel on the right shows its options: the document when nothing is selected,
pages when you select thumbnails, an object when you select one with the arrow tool. Right-click anything for a short
menu, or press Ctrl+K to search all commands.

See [Using the desktop app](gui.md).

## The command line

```
mpe info report.pdf
mpe merge a.pdf b.pdf -o both.pdf
mpe extract report.pdf -o summary.pdf --pages 1-3,7
mpe ocr scan.pdf -o searchable.pdf
mpe watermark report.pdf -o draft.pdf --text DRAFT --opacity 0.3
mpe form fill form.pdf -o filled.pdf --set name=Ann --set agree=on
```

See the [command-line reference](cli.md) for every command.
