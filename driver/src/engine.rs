//! The engine turns lua scripts (build plans) into derivations.

use std::cell::RefCell;
use std::path::Path;

use mlua::Lua;
use mlua::LuaOptions;
use mlua::StdLib;

use crate::drv::AnyDerivation;
use crate::drv::BuildDerivation;
use crate::drv::Derivation;
use crate::drv::FileDerivation;

pub fn run_script(script: impl AsRef<Path>) -> Result<Vec<AnyDerivation>, mlua::Error> {
    let lua = Lua::new_with(StdLib::STRING, LuaOptions::new()).expect("TODO");

    let derivations = RefCell::new(vec![]);

    lua.scope(|scope| {
        // TODO: make sure the borrow_mut() calls are safe; I _think_ there is no concurrent
        // stuff going on, so they probably should be.
        let file_drv =
            scope.create_function_mut(|_, (input_path, glob): (String, Option<String>)| {
                let digest = FileDerivation::expected_digest(&input_path, &glob)
                    .map_err(|err| {
                        mlua::Error::RuntimeError(format!("calculating digest: {}", err))
                    })?
                    .digest;

                let drv = FileDerivation {
                    input_path,
                    glob,
                    digest: digest.to_vec(),
                };
                let output_path = drv.output_path().to_str().unwrap().to_string();
                derivations.borrow_mut().push(AnyDerivation::File(drv));

                Ok(output_path)
            })?;
        lua.globals().set("file_drv", file_drv)?;

        let build_drv = scope.create_function_mut(|_, (builder,): (Vec<String>,)| {
            let drv = BuildDerivation { builder };
            let output_path = drv.output_path().to_str().unwrap().to_string();
            derivations.borrow_mut().push(AnyDerivation::Build(drv));

            Ok(output_path)
        })?;
        lua.globals().set("build_drv", build_drv)?;

        lua.load(script.as_ref()).exec()
    })?;

    Ok(derivations.into_inner())
}

pub fn run_derivations(derivations: Vec<AnyDerivation>) -> std::io::Result<()> {
    for drv in derivations {
        drv.run()?;
    }

    Ok(())
}
