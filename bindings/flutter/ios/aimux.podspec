Pod::Spec.new do |s|
  s.name             = 'aimux'
  s.version          = '0.2.0'
  s.summary          = 'Flutter binding for aimux — unified LLM service layer (Rust core, 325 providers, dart:ffi C ABI)'
  s.description      = <<-DESC
Aimux is a Rust alternative to the Vercel AI SDK: a unified provider
interface for LLM applications. This package ships the aimux-ffi static
library (`aimux_ffi.xcframework`) and calls it directly from Dart via
dart:ffi.
  DESC
  s.homepage         = 'https://github.com/arcships/aimux'
  s.license          = { :type => 'MIT', :file => '../LICENSE' }
  s.author           = { 'aimux contributors' => 'ericted8810@gmail.com' }
  s.source           = { :path => '.' }
  s.platform         = :ios, '12.0'

  # Rust static library xcframework (device arm64 + simulator arm64 slices),
  # produced by the release pipeline and vendored here before
  # `dart pub publish`.
  #
  # Dart resolves symbols via DynamicLibrary.process(), so every slice must be
  # force-loaded — otherwise the linker drops unreferenced archive objects and
  # lookupFunction fails at runtime. The xcframework slice names follow
  # xcodebuild's convention (ios-arm64 / ios-arm64-simulator); CI verifies
  # them before publishing.
  # Static framework slices (framework-type binaries link reliably in
  # Xcode/SPM; bare .a slices did not reach the app link).
  s.vendored_frameworks = 'aimux/Sources/aimux_ffi.xcframework'
  s.pod_target_xcconfig = {
    'OTHER_LDFLAGS[sdk=iphoneos*]' => ['-force_load', '$(PODS_TARGET_SRCROOT)/aimux/Sources/aimux_ffi.xcframework/ios-arm64/aimux_ffi.framework/aimux_ffi'],
    'OTHER_LDFLAGS[sdk=iphonesimulator*]' => ['-force_load', '$(PODS_TARGET_SRCROOT)/aimux/Sources/aimux_ffi.xcframework/ios-arm64-simulator/aimux_ffi.framework/aimux_ffi']
  }

  # App Store privacy manifest (2024-05+ requirement for SDKs). aimux-ffi
  # uses no required-reason APIs — the manifest is an explicit empty
  # declaration, bundled for both CocoaPods and SwiftPM consumers.
  s.resource_bundles = {
    'aimux_privacy' => ['aimux/Sources/aimux/PrivacyInfo.xcprivacy']
  }
end
