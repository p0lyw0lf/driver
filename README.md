# PolyWolf's Build [Driver](https://github.com/p0lyw0lf/driver)

WIP

## What?

A ***🚀BLAZING🔥FAST💨*** Static Site Generator written in Rust that interprets
framework-less Javascript files using [QuickJS](https://bellard.org/quickjs/).

Features:
+ Full incremental rebuilds, including when build logic changes.
+ Remote inputs.
+ Built-in Markdown ([comrak](https://github.com/kivikakk/comrak)),
  HTML ([minify-html](https://github.com/wilsonzlin/minify-html)), and
  Image (TODO) support.

## Why?

Many times have I been saddened by Static Site Generators not being fast enough
for my tastes. The goal of this project is to simply be the fastest engine
around, especially for rebuilds. TODO: benchmarks

## How?

See [my website](https://github.com/p0lyw0lf/website) for an example of what
using this engine looks like. Sorry the docs are kinda non-existent otherwise
at the moment, but hey I wrote this project just for me really.

(At the time of writing, the example is actually on the `website3` branch of
that repository).
