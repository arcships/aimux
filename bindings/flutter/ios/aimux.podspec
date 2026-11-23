Pod::Spec.new do |s|
  s.name             = 'aimux'
  s.version          = '0.6.1'
  s.summary          = 'Flutter binding for aimux — unified LLM service layer (Rust core, 327 providers, dart:ffi C ABI)'
  s.description      = <<-DESC
Aimux is a Rust alternative to the Vercel AI SDK: a unified provider
interface for LLM applications. This package ships the aimux-ffi dynamic
framework (`aimux_ffi.xcframework`) and calls it directly from Dart via
dart:ffi.
  DESC
  s.homepage         = 'https://github.com/arcships/aimux'
  s.license          = { :type => 'MIT', :file => '../LICENSE' }
  s.author           = { 'aimux contributors' => 'ericted8810@gmail.com' }
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.preserve_paths   = '../native/aimux_dart.c'
  s.swift_version    = '5.0'
  # Keep the aimux plugin pod itself static so the small aimux_dart_* C bridge
  # compiles into the host app binary and resolves via DynamicLibrary.process().
  s.static_framework = true
  s.dependency       'Flutter'
  s.platform         = :ios, '12.0'

  # Dynamic framework slices (device arm64 + simulator arm64), produced by
  # scripts/build-ios-xcframework.sh and vendored here before
  # `dart pub publish`.
  #
  # NOTE: Flutter's SwiftPM integration does not link plugin binary targets
  # (as of Flutter 3.44), so iOS integration goes through CocoaPods — the
  # official fallback. Flutter automatically uses it for podspec-only
  # plugins.
  #
  # Because aimux_ffi.xcframework contains genuine MH_DYLIB slices, CocoaPods
  # stages the matching slice via `[CP] Copy XCFrameworks`, embeds and signs
  # `aimux_ffi.framework` into `Runner.app/Frameworks/` via `[CP] Embed Pods
  # Frameworks`, and Dart loads it via
  # `DynamicLibrary.open('aimux_ffi.framework/aimux_ffi')`.
  s.vendored_frameworks = 'aimux_ffi.xcframework'

  # Ensure the host app searches Runner.app/Frameworks at runtime and retains
  # the aimux_dart_* C bridge symbols in the main executable for
  # DynamicLibrary.process().
  s.user_target_xcconfig = {
    'LD_RUNPATH_SEARCH_PATHS' => '$(inherited) @executable_path/Frameworks',
    'OTHER_LDFLAGS' => [
      '-Wl,-u,_aimux_dart_finalize', '-Wl,-u,_aimux_dart_token_new',
      '-Wl,-u,_aimux_dart_token_dispose', '-Wl,-u,_aimux_dart_bridge_new',
      '-Wl,-u,_aimux_dart_bridge_event', '-Wl,-u,_aimux_dart_bridge_ack',
      '-Wl,-u,_aimux_dart_bridge_cancel', '-Wl,-u,_aimux_dart_bridge_pending',
      '-Wl,-u,_aimux_dart_bridge_failure', '-Wl,-u,_aimux_dart_bridge_free',
      '-Wl,-u,_aimux_dart_frame_free',
    ],
  }

  # App Store privacy manifest (2024-05+ requirement for SDKs). aimux-ffi
  # uses no required-reason APIs — the manifest is an explicit empty
  # declaration.
  s.resource_bundles = {
    'aimux_privacy' => ['PrivacyInfo.xcprivacy']
  }
end
