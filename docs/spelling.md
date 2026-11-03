[User guide](README.md) › Everyday tasks

# Checking spelling

LimarePDF checks your spelling as you type, as a word processor does, in Canadian, US, British or
Australian English, or in French. It comes with those dictionaries, and you can add others.

## As you type

A word spelt wrongly gets a red wavy line under it once you finish it (type a space or a
punctuation mark after it). This works everywhere you type the document's words: text you edit
or add on the page, a paragraph being edited, a text box, a note, a comment or reply in the
inspector, and a form field. The marks show while you type there: once you finish editing a line,
a paragraph, a text box or a note, they go, and words on the page aren't marked while you read.

Right-click a marked word for:

- **Suggestions**, at the top of the menu: pick one to put it in place of the word. Undo takes
  it back, as with anything you type.
- **Ignore all**: leave this word, wherever it is, until LimarePDF closes.
- **Add to dictionary**: never mark this word again. Added in small letters ("limarepdf"), it is
  right in any capitals; added with a capital ("Limare"), only as written.

LimarePDF leaves out, as word processors do, words in capitals ("PDF"), words with numbers
("3rd"), and web, email and file addresses.

To stop the marks, turn off **Review > Spelling ▾ > Check spelling as you type**. The choice is
remembered.

## Go through the document

**Review > Spelling** (F7, or Option+Cmd+L on a Mac) opens the Spelling view in the side panel.
It goes through the words spelt wrongly in what was written in the document: its comments,
replies, notes, text boxes and form fields, one word at a time. Each word is shown in its
sentence, with what it is in and its page, and the page shows where it is.

- **Change** puts the suggestion chosen in its place; double-clicking a suggestion does the same.
- **Change all** changes every one of that word in the document, keeping each one's capital
  letter, as one step that Undo takes back.
- **Ignore** leaves this one; **Ignore all** leaves every one until LimarePDF closes.
- **Add to dictionary** never marks the word again.

When there are none left, the view says so. The pages' own text (what was printed into the PDF)
isn't gone through; to fix a word there, edit its line or paragraph, where the marks show as you
type.

## Choose the language

**Review > Spelling ▾** lists the languages there are: English (Canada), English (United
States), English (United Kingdom), English (Australia) and French, then any you added. The one
ticked is checked, for every document, and is remembered. Until you choose one, LimarePDF uses
the document's own language, when it names one there is a dictionary for, then your computer's,
then US English.

The Canadian dictionary has Canadian spelling as it stands: "colour", "centre", and "organize"
with a z. If you write "organise", choose English (United Kingdom).

The French dictionary is French as written everywhere, Canada included ("courriel", "magasinage").

## Add a dictionary

For another language, **Review > Spelling ▾ > Add a dictionary…** takes:

- a LibreOffice dictionary extension (an `.oxt` file, from LibreOffice's extensions site), or
- a Hunspell dictionary's two files, picked together: the `.dic` file and the `.aff` file of
  the same name.

LimarePDF keeps a copy in the `dictionaries` folder beside its settings, and lists the language
in the menu; choose it there. On Linux, the dictionaries installed for the whole system (in
`/usr/share/hunspell`, for example by a `hunspell-de` package) are listed too, without adding
them. A dictionary LimarePDF can't read is not added, and it says why.

---

[← Editing text and pictures](content-editing.md) · [User guide](README.md) · [Filling in forms →](forms.md)
