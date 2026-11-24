#!/bin/sh
# Invoke a plugin action through Herdr and wait for its result.
case "$0" in
  */*) dir=${0%/*} ;;
  *) dir=. ;;
esac
exec "$dir/../target/release/herdr-sbx" run-action "$@"
