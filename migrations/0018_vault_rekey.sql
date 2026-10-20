-- A changed vault password brings a new master key. Until every sealed file
-- has been re-sealed under it, the old master key is kept here, sealed under
-- the new one, so either password-change survivor can still be read; once
-- the last file is done both columns go back to NULL. The old password opens
-- neither: it only ever opened the old `sealed`, which is gone.
ALTER TABLE vault ADD COLUMN retired_nonce BLOB;
ALTER TABLE vault ADD COLUMN retired_sealed BLOB;
