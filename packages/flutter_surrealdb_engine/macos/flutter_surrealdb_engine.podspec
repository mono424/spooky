#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint flutter_surrealdb_engine.podspec` to validate before publishing.
#
Pod::Spec.new do |s|
  s.name             = 'flutter_surrealdb_engine'
  s.version          = '0.0.1'
  s.summary          = 'A new Flutter plugin project.'
  s.description      = <<-DESC
A new Flutter plugin project.
                       DESC
  s.homepage         = 'http://example.com'
  s.license          = { :file => '../LICENSE' }
  s.author           = { 'Your Company' => 'email@example.com' }

  s.source           = { :path => '.' }
  s.source_files = 'Classes/**/*'

  # If your plugin requires a privacy manifest, for example if it collects user
  # data, update the PrivacyInfo.xcprivacy file to describe your plugin's
  # privacy impact, and then uncomment this line. For more information,
  # see https://developer.apple.com/documentation/bundleresources/privacy_manifest_files
  # s.resource_bundles = {'flutter_surrealdb_engine_privacy' => ['Resources/PrivacyInfo.xcprivacy']}

  s.dependency 'FlutterMacOS'
  s.library = 'c++'

  s.platform = :osx, '10.11'
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES', 'OTHER_LDFLAGS' => '-lc++' }
  s.swift_version = '5.0'

  s.script_phase = {
    :name => 'Build Rust library',
    :script => 'sh "$PODS_TARGET_SRCROOT/../rust_builder/cargokit/build_pod.sh" ../rust rust_lib_flutter_surrealdb_engine',
    :execution_position => :before_compile,
    :input_files => ['${PODS_TARGET_SRCROOT}/../rust_builder/cargokit/src/cargokit.rs'],
    :output_files => ['${BUILT_PRODUCTS_DIR}/cargokit_phony']
  }

end
