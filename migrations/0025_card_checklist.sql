-- What a card holds besides its title, the way a card on a paper board does:
-- a checklist of the steps it takes, and labels that say what kind of work
-- it is. Both are small, read with the card and written with it, so they are
-- JSON on the card rather than tables of their own: `checklist` is an array
-- of `{"text": …, "done": …}`, `labels` an array of names. The description is
-- the card's `body`, which it has had from the start.
ALTER TABLE cards ADD COLUMN checklist TEXT NOT NULL DEFAULT '[]';
ALTER TABLE cards ADD COLUMN labels TEXT NOT NULL DEFAULT '[]';
