{
  lib,
  rustPlatform,
  pkg-config,
  sqlite,
  testers,
}:

rustPlatform.buildRustPackage {
  pname = "sifter";
  version = "0.1.0";
  src = lib.cleanSource ./.;

  cargoLock = {
    lockFile = ./Cargo.lock;
  };

  nativeBuildInputs = [pkg-config];
  buildInputs = [sqlite];

  cargoTestFlags = ["--workspace"];

  passthru.tests.version = testers.testVersion {
    package = rustPlatform.buildRustPackage {
      pname = "sifter";
      version = "0.1.0";
      src = lib.cleanSource ./.;
      cargoLock = {
        lockFile = ./Cargo.lock;
      };
    };
    command = "sifter --version";
  };

  meta = {
    description = "Local-first search engine for code and documentation";
    homepage = "https://github.com/jonochang/sifter";
    license = lib.licenses.mit;
    mainProgram = "sifter";
    platforms = lib.platforms.unix;
  };
}
