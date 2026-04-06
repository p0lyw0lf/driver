{
  pkgs,
  rust-toolchain,
  ...
}:
pkgs.mkShell {
  buildInputs = [
    rust-toolchain
  ]
  ++ (with pkgs; [
    pkg-config
    openssl
  ]);
  nativeBuildInputs = with pkgs; [
    # For debugging
    vscode-extensions.vadimcn.vscode-lldb.adapter
    # For developing
    cargo-watch
  ];
}
