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
use crate::drv::FileInput;

pub fn run_script(script: impl AsRef<Path>) -> Result<Vec<AnyDerivation>, mlua::Error> {
    let lua = Lua::new_with(StdLib::STRING, LuaOptions::new()).expect("TODO");

    let derivations = RefCell::new(vec![]);

    lua.scope(|scope| {
        // TODO: make sure the borrow_mut() calls are safe; I _think_ there is no concurrent
        // stuff going on, so they probably should be.
        let file_drv = scope.create_function_mut(|_, (path, glob): (String, Option<String>)| {
            let input = FileInput { path, glob };
            let digest = input
                .digest()
                .map_err(|err| mlua::Error::RuntimeError(format!("calculating digest: {}", err)))?;

            let drv = FileDerivation {
                input,
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

        let glob = scope.create_function(|_, (path, glob): (String, String)| {
            let input = FileInput {
                path,
                glob: Some(glob),
            };
            let files = input
                .files()
                .map_err(|err| mlua::Error::RuntimeError(format!("collecting files: {}", err)))?
                .into_iter()
                .map(|f| f.to_str().unwrap().to_string())
                .collect::<Vec<_>>();

            Ok(files)
        })?;
        lua.globals().set("glob", glob)?;

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
