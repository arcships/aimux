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

  # Symbol placement (consulted gpt-5.6-sol + glm-5.2, issue #25):
  # aimux is a DYNAMIC framework under use_frameworks, and its link happens
  # in the pod target — pod_target_xcconfig's force_load put the Rust
  # symbols into aimux.framework, which is never embedded into the app
  # (Runner.app/Frameworks is empty), so nothing is visible at runtime.
  # Dart resolves via DynamicLibrary.process() (dlsym RTLD_DEFAULT): symbols
  # must live in the MAIN executable. user_target_xcconfig reaches Runner's
  # link (Pods xcconfig is consumed — OTHER_LDFLAGS shows -framework aimux).
  # No -framework aimux_ffi here: force_load with an absolute path is
  # self-sufficient, and avoids duplicate definitions if the pod link also
  # picked it up.
  s.user_target_xcconfig = {
    'OTHER_LDFLAGS' => [
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
