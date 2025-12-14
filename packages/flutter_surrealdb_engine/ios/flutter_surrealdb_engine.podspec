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
  s.dependency 'Flutter'
  s.library = 'c++'
  s.platform = :ios, '13.0'

  # Flutter.framework does not contain a i386 slice.
  # Flutter.framework does not contain a i386 slice.
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES', 'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386' }
  s.swift_version = '5.0'

  s.script_phase = {
    :name => 'Build Rust library',
    :script => 'sh "$PODS_TARGET_SRCROOT/../rust_builder/cargokit/build_pod.sh" ../rust rust_lib_flutter_surrealdb_engine',
    :execution_position => :before_compile,
    :input_files => ['${PODS_TARGET_SRCROOT}/../rust_builder/cargokit/src/cargokit.rs'],
    :output_files => ['${BUILT_PRODUCTS_DIR}/cargokit_phony']
  }
end
