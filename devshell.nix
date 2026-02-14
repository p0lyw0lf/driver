{
  pkgs,
  rust-toolchain,
  ...
}:
pkgs.mkShell {
  buildInputs = [ rust-toolchain ];
  nativeBuildInputs = with pkgs; [
    # For running derivations
    python3
    # For debugging
    vscode-extensions.vadimcn.vscode-lldb.adapter
    # For developing
    cargo-watch
  ];
}
