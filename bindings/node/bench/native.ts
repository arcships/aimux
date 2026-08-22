// Path of the napi binary for the current platform, relative to this bench
// directory. Follows napi-rs artifact naming:
// aimux.<platform>-<arch>[-<abi>].node (e.g. aimux.linux-x64-gnu.node,
// aimux.darwin-arm64.node, aimux.win32-x64-msvc.node).
export function nativeBinaryPath(): string {
  const abi =
    process.platform === 'linux' ? '-gnu' : process.platform === 'win32' ? '-msvc' : ''
  return `../aimux.${process.platform}-${process.arch}${abi}.node`
}
