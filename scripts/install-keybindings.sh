#!/bin/sh
# Install the plugin key bindings in the Herdr configuration.
case "$0" in
  */*) dir=${0%/*} ;;
  *) dir=. ;;
esac
exec "$dir/../target/release/herdr-sbx" install-keybindings
