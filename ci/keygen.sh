#!/bin/sh
cd $(dirname $0)

if [ "$1" = "" ] || [ "$2" = "" ]
then
    echo "Usage: keygen.sh [DOMAIN] [IP]"
    exit 1
fi

export DOMAIN="$1"
export IP="$2"
export CA_NAME=${CA_NAME:-"rust-socketio-dev"}

mkdir cert || true

cargo run --manifest-path keygen/Cargo.toml