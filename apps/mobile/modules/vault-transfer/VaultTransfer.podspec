require 'json'
Pod::Spec.new do |s|
  s.name = 'VaultTransfer'
  s.version = '0.1.0'
  s.summary = 'Durable native audio preparation and background tus uploads'
  s.description = s.summary
  s.license = { :type => 'MIT' }
  s.author = 'Transcript Vault'
  s.homepage = 'https://example.invalid/transcript-vault'
  s.platform = :ios, '17.0'
  s.swift_version = '6.0'
  s.source = { :git => 'https://example.invalid/transcript-vault.git', :tag => s.version.to_s }
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  s.source_files = 'ios/**/*.swift'
  s.frameworks = 'AVFoundation', 'Security', 'Combine'
end
