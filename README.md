# PolyWolf's Build [Driver](https://github.com/p0lyw0lf/driver)

## What?

A ***🚀BLAZING🔥FAST💨*** Static Site Generator written in Rust that interprets
framework-less Javascript files using [BoaJS](https://boajs.dev/).

Features:
+ Full incremental rebuilds, including when build logic changes.
+ Remote inputs.
+ Built-in support for handling:
  + Markdown ([comrak](https://github.com/kivikakk/comrak))
  + HTML ([minify-html](https://github.com/wilsonzlin/minify-html))
  + Images ([zune-image](https://github.com/etemesi254/zune-image))

## Why?

Many times have I been saddened by Static Site Generators not being fast enough
for my tastes. The goal of this project is to simply be the fastest engine
around, especially for rebuilds. As a comparison to the engine I used previously,
<https://astro.build>:

| **What**        | Astro | Driver | Speedup |
|-----------------|------:|-------:|--------:|
| Clean Build     | ~1min | ~1min  | 1x      |
| Template Change | ~20s  | ~500ms | 40x     |
| Post Change     | ~20s  | ~200ms | 100x    |
| No Change       | ~20s  | ~50ms  | 400x    |

Most of the time in Clean Build is taken up by an image transformation pipeline.
Do note these are very unscientific measurements, taken just from my personal
website, on my personal laptop.

## How?

See my [website](https://github.com/p0lyw0lf/website) for an example of what
using this engine looks like, and [website-template](https://github.com/p0lyw0lf/website-template)
for a bit more of an opinionated example Sorry the docs are kinda sparse at the
moment, but hey I wrote this project just for me really.

## Read More

I've posted about this a few times on my blog:

- 2026-06-16: [Async Task Locals From Scratch](https://wolfgirl.dev/blog/2026-06-16-async-task-locals-from-scratch/)
- 2026-04-13: [NEW BLOG ENGINE WORKING!!](https://wolfgirl.dev/blog/2026-04-13-new-blog-engine-working-/)
- 2026-02-23: [So I've Been Thinking About Static Site Generators](https://wolfgirl.dev/blog/2026-02-23-so-ive-been-thinking-about-static-site-generators/)
