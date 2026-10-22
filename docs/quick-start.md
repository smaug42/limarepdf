# Quick start

## The desktop app

Open a file with the Open button, Ctrl+O, or by dropping it on the window, or from a terminal:

```
limarepdf report.pdf --page 3       # open at page 3
limarepdf scan.tif photo.jpg        # images open as a new document, one page per image
```

Click what you want to work on, and the panel on the right shows its options: the document when nothing is selected,
pages when you select thumbnails, a picture, shape or comment when you click it, a line of text when you double-click
it to edit it. Right-click anything for a short
menu, or press Ctrl+K to search all commands.

See [Using the desktop app](gui.md).

## The command line

```
limarepdf-cli info report.pdf
limarepdf-cli merge a.pdf b.pdf -o both.pdf
limarepdf-cli extract report.pdf -o summary.pdf --pages 1-3,7
limarepdf-cli ocr scan.pdf -o searchable.pdf
limarepdf-cli watermark report.pdf -o draft.pdf --text DRAFT --opacity 0.3
limarepdf-cli form fill form.pdf -o filled.pdf --set name=Ann --set agree=on
```

See the [command-line reference](cli.md) for every command.
