[User guide](README.md) › Help

# Troubleshooting

Common problems and what to do about them. If yours isn't here, [report it](#reporting-a-problem).

## Installing and starting

### The system warns about the app the first time

LimarePDF isn't code-signed yet, so Windows and macOS warn before the first start:

- **Windows** (SmartScreen): choose More info, then Run anyway.
- **macOS** ("Apple cannot check it"): choose Done, open System Settings > Privacy & Security, choose Open Anyway
  next to LimarePDF, and confirm. For the archive, you can instead run
  `xattr -d com.apple.quarantine limarepdf-cli limarepdf` in the unpacked folder.

To check that a download really comes from this project, see [Checking a download](install.md#checking-a-download).

### The app doesn't start on Linux

The desktop app needs a working OpenGL graphics driver and an X11 or Wayland desktop session. Start `limarepdf` from
a terminal to see why it stopped, and [report it](#reporting-a-problem) with that message.

### The window has a plain icon on Linux

Wayland desktops take a window's icon from its menu entry. The packages install one; for the archive, see
[Menu entry and icon on Linux](install.md#menu-entry-and-icon-on-linux).

## Opening documents

### Dropping files on the window does nothing

On Linux in a Wayland session (the default on current KDE and GNOME), the pointer shows a "no" sign: the windowing
library LimarePDF uses doesn't take file drops from Wayland yet. Open files with the Open button or Ctrl+O instead;
dragging pages within the app works too.

To have drops, start LimarePDF through XWayland, Wayland's X11 compatibility layer, with
`env -u WAYLAND_DISPLAY limarepdf`. With fractional display scaling (such as 125% or 150%) the window can then look
blurry, unless your desktop lets X11 apps scale themselves (in KDE, System Settings > Display & Monitor > Legacy
Applications (X11)). To make it the default, copy LimarePDF's menu entry (`io.github.smaug42.limarepdf.desktop`,
usually in `/usr/share/applications`) to `~/.local/share/applications` and change its `Exec=` line to start with
`env -u WAYLAND_DISPLAY`.

### "The file is damaged" notice

LimarePDF repaired the file to open it, and some content may be missing. Your file isn't changed: the document
counts as changed, and Save asks whether to replace the original or save the repaired copy as a new file. Saving a
copy is the safe choice. A signed file's signatures can't vouch for a repaired copy.
[More](rendering.md#damaged-files).

### Text is missing, or shows in a different font

The document uses fonts it doesn't include, and LimarePDF draws them with similar fonts installed on your
computer. A notice names them, and for Chinese, Japanese or Korean text says which free font to install (such as
Noto Sans CJK JP for Japanese). Install it, then quit and restart LimarePDF, which reads the installed fonts once
per run. [More](rendering.md#fonts-that-arent-embedded).

### The page says "Please wait..."

It is a form made for Adobe Reader (an XFA form), which only Adobe software can show or fill in. A notice says so;
what you see is the placeholder page the file contains.

### It asks for a password

The document is protected with an open password: you need it from whoever sent the document. Some documents open
without one but have a permissions password, which limits printing, copying or changes. See
[Password protection](security.md).

## Typing and editing

### "LimarePDF can't add this character yet"

New and edited text can be in Latin (Vietnamese included), Greek and Cyrillic, and Chinese, Japanese, Korean and
single emoji in an installed font that has them. Hebrew, Arabic and scripts whose letters join (Devanagari, Thai
and others) can't be written yet, in any font. Text boxes, headers, footers, watermarks and fill-and-sign text are
written in Helvetica, so they take only Latin, Greek and Cyrillic: for Chinese, Japanese, Korean or single emoji,
use the Add text tool in an installed font that has them. [What doesn't work yet](features.md#what-doesnt-work-yet) has the details.

### "LimarePDF can't add this emoji yet"

Emoji made of several characters can't be written yet, in any font: flags, emoji with a skin tone, keycaps (1️⃣)
and joined emoji such as families. An emoji picker or phone often adds an invisible mark that asks for the
colourful look (the emoji variation selector); that counts too. A single emoji without these can be typed in an
installed font that has it, such as Symbola or DejaVu Sans.

### "Helvetica can't write this character; choose an installed font that has it"

Helvetica, Times and Courier have no Chinese, Japanese or Korean, and no emoji (and lack some other characters).
Choose an installed font that has them in the inspector's font list, such as Noto Sans CJK, or Symbola for emoji,
then type again.

### "… has no …; choose another font"

The installed font you chose doesn't have that character, or draws it only in colour (some fonts have colour
emoji), which LimarePDF can't write into a PDF. Choose another font in the inspector's font list.

### "LimarePDF can't draw this text box again"

The text box was made by another app and has characters Helvetica can't write (Chinese, Hebrew and so on).
LimarePDF draws text boxes in Helvetica, so it keeps the box's own drawing and refuses a new colour, opacity,
style, size or text rather than draw those characters as `?`. You can still move or delete it. To change its
look, edit it in the app that made it. Chinese, Japanese, Korean and single emoji can also be written with the
Add text tool in an installed font that has them; Hebrew, Arabic and other right-to-left or shaped scripts, emoji
made of several characters and emoji drawn only in colour can't be written yet. Add the new text first and check
it, then delete the old box, so nothing is lost if some characters can't be written.

### A text box shows `?` instead of some characters

A text box made by another app without a drawing of its own is drawn by LimarePDF in Helvetica, on screen, when
printed and in exported images, so characters Helvetica can't write come out as `?` there. The file isn't
changed, and other readers may show the box as its app made it. Flattening comments refuses such a box, naming its
page ("LimarePDF can't draw the text box on page …"), rather than draw `?` into the page for good. Text boxes made
in LimarePDF never show `?`: what Helvetica can't write isn't typed.

### An edited line looks different

When LimarePDF can't keep a line's own font (common for fonts embedded in the file), it writes the line in the
installed font it came from, or in Helvetica, Times or Courier. The inspector says so. See
[Editing text and pictures](content-editing.md#limits).

### Undo doesn't go back far enough

Undo keeps up to 20 steps, and at most 1 GB per document, so very large documents get fewer steps.

## Saving and recovery

### Saving removed the document's password

A protected document is saved without its password unless you set it again: a notice says so when it opens. Set
it under Password security in the inspector before saving.

### "The file was saved, but your system couldn't confirm it is safely on disk"

The save worked, but the system couldn't confirm it is safely on the disk. If the computer lost power just then,
save again; the document stays marked unsaved so that is one click. When this happens while closing or quitting,
the document stays open, and closing or quitting stops, so you can save it again first.

### Settings not saved

LimarePDF couldn't write to its settings folder (see [Where LimarePDF keeps things](#where-limarepdf-keeps-things)),
usually because the folder or a file in it can't be written to. What you changed (the theme, your comment name,
a signature) works until LimarePDF closes, but isn't there next time. The message comes once a session; one about
a signature saved for next time comes each time. Check that the folder is yours and that the disk isn't full.

"Settings not confirmed" means they were saved, but your system couldn't confirm they are safely on the disk:
if the computer lost power just then, the latest changes may be lost.

A digital ID made or imported that couldn't be made the default is still in your IDs; don't make or import it
again. Choose **Make default** later, or pick it when you sign.

### LimarePDF closed unexpectedly

At the next start, LimarePDF lists the documents that had unsaved changes. Restore opens each as a new document
called "*name* (recovered)"; save it where you want it. Changes from the last minute or so before the crash can be
missing. [Recovery copies](gui.md#recovery-copies) explains how this works.

### "Recovery copies of it can't be kept"

Usually the disk is nearly full (LimarePDF leaves at least 1 GB free). Save the document, and free some space.

## Printing

### The printout looks different from the screen

On Linux and macOS the pages go to the printing system (CUPS), which draws them itself. Turn on Print as image,
under Advanced in Print, to send the pages as LimarePDF draws them.

### No printers are listed

On Linux and macOS, LimarePDF prints through CUPS (macOS includes it): add your printer in the system's printer
settings, then press Try again in Print. Printing on paper has only been tried on Linux so far.

### Print says the printing service didn't answer

LimarePDF waits 10 seconds for the system to list the printers. On Linux, check that CUPS is running (for example
with `systemctl status cups`), then press Try again.

## Digital signatures

### A signature shows "not confirmed" or "NOT TRUSTED"

LimarePDF only trusts signers you have told it to trust. Compare the certificate's fingerprint (in Details) with
the signer, then choose Trust this signer…, or trust their organisation's authority. See
[Trusting signers](signatures.md#trusting-signers).

### I forgot my digital ID's password

There is no way back in, by design. Make a new digital ID, and send the people who trusted the old one your new
certificate. Keep a backup of each ID (Export) with a password you will remember.

More messages and what they mean are in [Digital signatures: troubleshooting](signatures.md#troubleshooting).

## Where LimarePDF keeps things

Everything stays on your computer.

| What | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Settings, recent files, saved signatures, trusted certificates, digital IDs | `~/.config/limarepdf/` | `~/Library/Application Support/limarepdf/` | `%APPDATA%\limarepdf\` |
| Recovery copies | `~/.local/state/limarepdf/recovery` | `~/Library/Application Support/limarepdf/recovery` | `%LOCALAPPDATA%\limarepdf\recovery` |

In the settings folder:

- `settings.txt`: your choices in the app (theme, page colours, markup colours, your name for comments, panels,
  window size and the like). To reset them, quit LimarePDF and delete this file.
- `recent.txt`: the recently opened files.
- `signatures.txt` and the `signatures` folder: your saved signatures and initials for Fill and sign.
- `trust.txt`: the signers and authorities you trust.
- `ids`: your digital IDs. Keep this folder private, and keep a backup of each ID (see
  [Digital IDs](signatures.md#digital-ids)).

On Linux, `$XDG_CONFIG_HOME` and `$XDG_STATE_HOME` move these folders when they are set.

## Reporting a problem

Report problems and suggestions on GitHub: [github.com/smaug42/limarepdf/issues](https://github.com/smaug42/limarepdf/issues).
It helps to include:

- what you did, what you expected, and what happened;
- the version and system: ☰ > Help > About, then Copy next to the version;
- the message, if there was one: a message's Copy button copies it;
- a document that shows the problem, if it isn't private. Never attach a private document; a made-up one that
  shows the same problem is just as useful.

A security problem, such as a signature shown as valid on a changed document or redaction leaving something
behind, is reported privately instead, as the [security policy](../SECURITY.md) describes.

---

[← Licence](licence.md) · [User guide](README.md) · [Glossary →](glossary.md)
