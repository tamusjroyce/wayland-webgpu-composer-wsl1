#!/bin/sh
# Fetch the Smithay library the WSL1 compositor builds against. Smithay is not
# vendored in this repo; it is cloned into ./smithay (git-ignored). Pinned to a
# known-good commit so local and CI builds are reproducible.
set -e

SMITHAY_COMMIT=118e34ffc9b99854a2230c4805e1f37dc9029edb

if [ ! -d smithay/.git ]; then
	git clone https://github.com/Smithay/smithay.git smithay
fi
git -C smithay fetch origin "$SMITHAY_COMMIT"
git -C smithay checkout "$SMITHAY_COMMIT"