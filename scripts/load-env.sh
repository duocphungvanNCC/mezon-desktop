#!/usr/bin/env bash
set -a
if [ -f .env ]; then
    # shellcheck disable=SC1091
    . ./.env
fi
set +a
