-- A card that comes back: `daily`, `weekly`, `monthly`, `yearly`, or every
-- `+Nd`, `+Nw`, `+Nm`, `+Ny`. Finishing such a card leaves a copy in the
-- inbox with the next due date and clears the rule on the finished one, so
-- reopening it and finishing it again does not make a second copy.
ALTER TABLE cards ADD COLUMN repeat TEXT;
