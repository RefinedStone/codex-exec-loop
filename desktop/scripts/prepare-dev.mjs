// `npm run dev` 전에(predev) 개발 실행에 필요한 두 가지를 준비한다.
// 1. Electron 바이너리: 저장소 검사(check_node_surfaces.sh)는 `ELECTRON_SKIP_BINARY_DOWNLOAD=1 npm ci` 로
//    node_modules 를 다시 설치하므로, 검사 뒤에는 바이너리가 없어 `Electron uninstall` 오류가 난다.
// 2. akra-gateway: 앱은 ../target 의 빌드를 띄운다. cargo 로 빌드해서 Rust 변경이 바로 반영되게 한다.
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** electron 패키지의 install.js 가 바이너리를 내려받아 path.txt 를 남겼는지 본다. */
export function electronBinaryReady(electronRoot, exists = existsSync, readText = readUtf8) {
  const pathFile = join(electronRoot, "path.txt");
  if (!exists(pathFile)) {
    return false;
  }
  return exists(join(electronRoot, "dist", readText(pathFile).trim()));
}

/** Gateway 를 빌드하지 않아도 되는 이유를 돌려준다. 빌드해야 하면 null 이다. */
export function gatewayBuildSkipReason(env) {
  if (env.AKRA_GATEWAY_ENDPOINT) {
    return "AKRA_GATEWAY_ENDPOINT is set; the app uses that gateway";
  }
  if (env.AKRA_GATEWAY_BIN) {
    return "AKRA_GATEWAY_BIN is set; the app starts that executable";
  }
  return null;
}

function readUtf8(path) {
  return readFileSync(path, "utf8");
}

function ensureElectronBinary(desktopRoot) {
  const electronRoot = join(desktopRoot, "node_modules", "electron");
  if (electronBinaryReady(electronRoot)) {
    return;
  }
  console.log("prepare-dev: Electron binary is missing; downloading it");
  const env = { ...process.env };
  delete env.ELECTRON_SKIP_BINARY_DOWNLOAD;
  const result = spawnSync(process.execPath, [join(electronRoot, "install.js")], { stdio: "inherit", env });
  if (result.status !== 0 || !electronBinaryReady(electronRoot)) {
    console.error("prepare-dev: Electron download failed; run `npm ci` in desktop/ and try again");
    process.exit(1);
  }
}

function buildGateway(repositoryRoot) {
  const skipReason = gatewayBuildSkipReason(process.env);
  if (skipReason) {
    console.log(`prepare-dev: skip akra-gateway build (${skipReason})`);
    return;
  }
  console.log("prepare-dev: cargo build --bin akra-gateway");
  const result = spawnSync("cargo", ["build", "--bin", "akra-gateway"], { cwd: repositoryRoot, stdio: "inherit" });
  if (result.error) {
    // cargo 가 없어도 이미 빌드된 Gateway 나 PATH 의 Gateway 로 앱을 열 수 있다.
    console.warn(`prepare-dev: could not run cargo (${result.error.message}); using an existing akra-gateway`);
    return;
  }
  if (result.status !== 0) {
    console.error("prepare-dev: akra-gateway build failed");
    process.exit(result.status ?? 1);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  ensureElectronBinary(desktopRoot);
  buildGateway(resolve(desktopRoot, ".."));
}
