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
  s.dependency       'Flutter'
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
  # Dart resolves symbols via DynamicLibrary.process(), so the static
  # archive must be force-loaded — otherwise the linker drops unreferenced
  # objects and lookupFunction fails at runtime.
  s.vendored_frameworks = 'aimux_ffi.xcframework'

  # CocoaPods' own xcframework script is not mounted in Flutter projects
  # (base configuration conflict — see issue #25), so the selected slice is
  # staged into PODS_XCFRAMEWORKS_BUILD_DIR/aimux by this script_phase,
  # which runs as part of the pod target build.
  s.script_phase = {
    :name => 'Stage aimux_ffi xcframework slice',
    :script => 'bash "$PODS_TARGET_SRCROOT/embed-xcframework.sh"',
    :execution_position => :before_compile,
    :input_files => ['${PODS_TARGET_SRCROOT}/aimux_ffi.xcframework/Info.plist'],
    :output_files => ['${PODS_XCFRAMEWORKS_BUILD_DIR}/aimux/aimux_ffi.framework/aimux_ffi'],
  }

  # use_frameworks makes aimux a dynamic framework; the vendored static
  # library is linked INTO it at the POD target's link step. Nothing
  # references the Rust symbols (Dart resolves them at runtime via
  # DynamicLibrary.process()), so pod_target_xcconfig force-loads the
  # staged archive — the flags must apply to the pod's own link, not the
  # app's. The staged slice lives under PODS_XCFRAMEWORKS_BUILD_DIR (already
  # in the pod's FRAMEWORK_SEARCH_PATHS via CocoaPods' xcframework wiring).
  s.pod_target_xcconfig = {
    'OTHER_LDFLAGS' => [
      '-framework', 'aimux_ffi',
      '-force_load', '$(PODS_XCFRAMEWORKS_BUILD_DIR)/aimux/aimux_ffi.framework/aimux_ffi',
    ],
  }

  # App Store privacy manifest (2024-05+ requirement for SDKs). aimux-ffi
  # uses no required-reason APIs — the manifest is an explicit empty
  # declaration.
  s.resource_bundles = {
    'aimux_privacy' => ['PrivacyInfo.xcprivacy']
  }
end
