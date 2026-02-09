import { markdown_to_html, minify_html, write_output } from "io";

const [name] = ARGS;

const input = `
# Hello, World!

This is a cool test page I am writing in markdown.
I should probably have written it in a separate file but that's OK!

## My Page

my page
`;
const html = `
<!DOCTYPE html>
<html>
<head><title>Test Page</title></head>

<body>${markdown_to_html(input)}</body>
</html>
`;
const out = minify_html(html);

write_output(name, out);
