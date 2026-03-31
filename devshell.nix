{
  pkgs,
  rust-toolchain,
  ...
}:
pkgs.mkShell {
  buildInputs = [ rust-toolchain ];
  nativeBuildInputs = with pkgs; [
    # For debugging
    vscode-extensions.vadimcn.vscode-lldb.adapter
    # For developing
    cargo-watch
  ];
}
