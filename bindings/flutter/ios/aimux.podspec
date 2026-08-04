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
  s.source_files     = 'Classes/**/*'
  s.swift_version    = '5.0'
  s.platform         = :ios, '12.0'

  # Static framework slices (device arm64 + simulator arm64), produced by
  # scripts/build-ios-xcframework.sh and vendored here before
  # `dart pub publish`.
  #
  # NOTE: Flutter's SwiftPM integration does not link plugin binary targets
  # (as of Flutter 3.44), so iOS integration goes through CocoaPods — the
  # official fallback. Flutter automatically uses it for podspec-only
  # plugins.
  #
  # Dart resolves symbols via DynamicLibrary.process(), so every slice must
  # be force-loaded — otherwise the linker drops unreferenced archive
  # objects and lookupFunction fails at runtime. The xcframework slice
  # names follow xcodebuild's convention (ios-arm64 / ios-arm64-simulator);
  # CI verifies them before publishing.
  s.vendored_frameworks = 'aimux_ffi.xcframework'
  # user_target_xcconfig: the force_load must reach the APP link command;
  # pod_target_xcconfig only affects the pod's own target build.
  s.user_target_xcconfig = {
    'OTHER_LDFLAGS[sdk=iphoneos*]' => ['-force_load', '$(PODS_ROOT)/aimux/aimux_ffi.xcframework/ios-arm64/aimux_ffi.framework/aimux_ffi'],
    'OTHER_LDFLAGS[sdk=iphonesimulator*]' => ['-force_load', '$(PODS_ROOT)/aimux/aimux_ffi.xcframework/ios-arm64-simulator/aimux_ffi.framework/aimux_ffi']
  }

  # App Store privacy manifest (2024-05+ requirement for SDKs). aimux-ffi
  # uses no required-reason APIs — the manifest is an explicit empty
  # declaration.
  s.resource_bundles = {
    'aimux_privacy' => ['PrivacyInfo.xcprivacy']
  }
end
