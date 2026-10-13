# Features and status

**Status: 0.3, for testing.** Most testing has been on Linux. Annotations, forms, digital signatures and redaction
are not started.

## What MPE does

- **View:** tabs, thumbnails, continuous scrolling, zoom, and search with highlights on the page.
- **Organise pages:** drag thumbnails to reorder; rotate, insert, delete and extract pages; merge and split files.
- **Edit content:** move, resize, restyle and delete text, images and shapes; edit text; add text, shapes and images.
- **Stamp pages:** headers, footers, page numbers, Bates numbers and text watermarks.
- **Scans:** turn images into PDFs, shrink and straighten scanned pages, and add a searchable text layer with
  built-in OCR (Latin script).
- **Copy text,** including recognised text.
- **Protect:** open and permissions passwords, RC4 or AES encryption, permission flags.
- **Export and print:** pages as PNG or JPEG, text as a `.txt` file, and printing (through CUPS on Linux and macOS,
  the print spooler on Windows).
- **Command line:** everything above, for scripts and batches.
- **Private:** no network connections, accounts or licence checks.

## Feature status

**Done:** works, and is covered by tests or has been run. **Partial:** works within the limits noted. **No:** not
available.

| Area | Status | Notes |
| --- | --- | --- |
| Tabs, thumbnails, opening by drag and drop | Done | |
| Zoom, fit width, continuous scrolling, page navigation | Done | Single-page and facing-page layouts: No |
| Open at a page from the command line | Done | `mpe-gui file.pdf --page N` |
| Command palette, right-click menus, floating toolbar | Done | Toolbar customisation: No |
| Rendering | Partial | Some transparency groups are drawn wrongly, and annotations without an appearance stream are not drawn. [Details](rendering.md) |
| Search | Partial | Highlights can be a letter off in proportional fonts. [Details](search-and-text.md) |
| Select and copy text | Done | One page at a time; includes recognised text |
| Page operations: insert, delete, rotate, extract, reorder, crop | Done | Crop is in the command line only |
| Merge and split | Done | Bookmarks, forms and named destinations of appended files are not carried over. Split by file size: No |
| Undo and redo | Done | 20 levels. Password settings cannot be undone |
| Headers, footers, page numbers, dates, Bates numbers | Done | Helvetica and Latin-1 characters only |
| Watermarks | Partial | Text only |
| Document properties | Done | Title, author, subject, keywords, creator, producer |
| Password security | Done | All four ciphers and permission flags. [Things to know](security.md) |
| Opening protected files | Done | With the open (user) password |
| Content editing | Partial | Works on lines, not paragraphs; some fonts are replaced when edited. [Details](content-editing.md) |
| Importing images as pages | Done | PNG, JPEG, TIFF. [Details](scanning-and-ocr.md#importing-images) |
| Optimising scanned pages | Partial | Pages that are one image. [Details](scanning-and-ocr.md#optimising-scanned-pages) |
| OCR | Partial | Latin script; built-in recognition model. [Details](scanning-and-ocr.md#recognising-text-ocr) |
| Export pages as images, export text | Done | [Details](export-and-printing.md) |
| Printing | Partial | No paper size or scaling options; not yet tried with a real printer. [Details](export-and-printing.md#printing) |
| Driving scanners | No | Not planned: scan with your system's tool and import the images |
| Export to Word, Excel, CSV | No | |
| Annotations, comments, stamps, measuring | No | Existing annotations are shown if they have an appearance stream |
| Forms, form data, barcodes, JavaScript | No | |
| Links, layers, attachments, bookmarks | No | |
| Redaction | No | |
| Digital signatures, certificate encryption | No | |
| PDF/A, optimised saving | No | Saved files are pruned and compressed |
| Comparing documents | No | |
| Sessions, autosave, backups, preferences, custom shortcuts, translations | No | You are asked before closing unsaved work |
| XFA forms, XPS documents | No | |

## Known limits

- Header, footer, Bates and watermark text uses Helvetica and Latin-1 only; other characters print as `?`.
- Headers, footers and watermarks are added to the page content. They are not annotations and cannot be removed one
  by one later; use Undo before saving.
- Saving rewrites the whole file; incremental saving is not available.
- Saving removes the validity of digital signatures, as any rewrite does.
- Files the renderer cannot read show "Rendering…" on their pages; editing may still work.
