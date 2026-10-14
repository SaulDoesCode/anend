# Author corpus

Excerpts used by `core/examples/authors.rs` to train and test an author classifier.
Each file holds ten excerpts of about 5 KB, taken from evenly spaced places in the
author's works and separated by lines reading `* * *`. Chapter and book headings were
dropped, line breaks kept, non-ASCII characters removed; the Project Gutenberg headers
and licence text are not included.

| file | author | works |
|---|---|---|
| austen.txt | Jane Austen | *Emma* (1816), *Persuasion* (1818) |
| chesterton.txt | G. K. Chesterton | *The Man Who Was Thursday* (1908), *The Wisdom of Father Brown* (1914) |
| melville.txt | Herman Melville | *Moby Dick* (1851) |
| milton.txt | John Milton | *Paradise Lost* (1667) |
| shakespeare.txt | William Shakespeare | *Hamlet*, *Macbeth*, *Julius Caesar* (First Folio spelling) |
| whitman.txt | Walt Whitman | *Leaves of Grass* |

The texts are in the public domain. They come from the Project Gutenberg selections
distributed with NLTK (`nltk_data/packages/corpora/gutenberg.zip`), originally
from Project Gutenberg (https://www.gutenberg.org).
