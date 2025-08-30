local foo = build_drv({ "bash", "-c", "echo foo > $out/foo" })
local bar = build_drv({ "bash", "-c", "echo bar > $out/bar" })
local foobar = build_drv({ "bash", "-c", `cat {foo}/foo {bar}/bar > $out/foobar` })

local lua = file_drv("./scripts/test.luau");
local js = file_drv("./scripts", "*.js");
