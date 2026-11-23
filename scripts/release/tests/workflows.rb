require 'yaml'
require 'json'
require 'minitest/autorun'
require 'tmpdir'
require 'open3'

class ReleaseWorkflows < Minitest::Test
  DIR = File.expand_path('../../../.github/workflows', __dir__)
  def workflow(name)
    YAML.safe_load(File.read(File.join(DIR, name + '.yml')))
  end
  def commands(job)
    job.fetch('steps').map { |step| step['run'] }.compact.join("\n")
  end
  def edges(jobs, name)
    Array(jobs.fetch(name)['needs'])
  end
  def ancestors(jobs, name)
    edges(jobs, name).flat_map { |dep| [dep] + ancestors(jobs, dep) }.uniq
  end
  def test_workflows_separate_triggers_and_classify_stripped_tags
    %w[release release-crapts].each do |name|
      w = workflow(name)
      prefix = name == 'release' ? 'v' : 'crapts-v'
      assert_equal({'push' => {'tags' => [prefix + '[0-9]+.[0-9]+.[0-9]+*']}}, w.fetch(true))
      jobs = w.fetch('jobs')
      assert_includes commands(jobs['preflight']), 'check-tag.sh ' + (name == 'release' ? 'cargo-crap' : 'crapts')
      assert_includes commands(jobs['preflight']), 'check-core.sh'
      assert_equal({'contents'=>'read'},jobs['preflight']['permissions'])
      assert_includes edges(jobs, 'build'), 'preflight'
      assert_includes edges(jobs, 'release'), 'build'
      assert_equal({'contents'=>'write'},jobs['release']['permissions'])
      product = name == 'release' ? 'cargo-crap' : 'crapts'
      assert_equal './.github/workflows/_build.yml', jobs['build']['uses']
      inputs = jobs['build']['with']
      assert_equal [product, product, name == 'release' ? 'triage' : ''], inputs.values_at('package','binary','features')
      targets = JSON.parse(inputs.fetch('targets'))
      expected = %w[aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-msvc]
      expected << 'x86_64-unknown-linux-musl' if name == 'release-crapts'
      assert_equal expected.sort, targets.map { |v| v['target'] }.sort
      targets.each do |row|
        assert_equal %w[aarch64-unknown-linux-gnu x86_64-unknown-linux-musl].include?(row['target']), row['use_cross']
      end
      assert_includes commands(jobs['release']), 'SHA256SUMS'
      release = jobs['release']['steps'].find { |s| s['uses'].to_s.include?('action-gh-release') }.fetch('with')
      if name == 'release'
        refute_equal 'false',release['make_latest'].to_s
        assert_equal ['release'],edges(jobs, 'action-smoke')
        assert_equal ['action-smoke'],edges(jobs, 'move-v1')
        refute_includes ancestors(jobs,'move-v1'), 'publish'
      else
        assert_equal 'false',release['make_latest'].to_s
        assert_includes release['prerelease'],'needs.preflight.outputs.is_prerelease'
        assert_equal %w[is_prerelease major version], jobs['preflight']['outputs'].keys.sort
      end
    end
  end
  def test_prerequisite_failures_block_every_dependent_publication_and_promotion
    jobs = workflow('release-crapts').fetch('jobs')
    stages = %w[preflight build release archive-smoke publish-crates publish-npm npm-smoke promote-npm]
    stages.each_with_index do |name,i|
      assert_empty stages.take(i) - ancestors(jobs, name)
      assert_includes edges(jobs,name), stages[i-1] if i > 0
      refute_match(/always\(|failure\(|continue-on-error/, jobs.fetch(name).inspect)
    end
    assert_match(/needs\.preflight\.outputs\.is_prerelease == 'false'/,jobs['promote-npm']['if'])
    assert_equal %w[contents id-token], jobs['promote-npm']['permissions'].keys.sort
    %w[publish-npm promote-npm].each do |name|
      assert_equal({'contents'=>'read','id-token'=>'write'}, jobs[name]['permissions'])
      refute_match(/NODE_AUTH_TOKEN|NPM_TOKEN/, jobs[name].inspect)
      assert_includes commands(jobs[name]),'npm@11.21.0'
    end
    other = workflow('release')['jobs']['publish']
    [other,jobs['publish-crates']].each do |job|
      assert_equal({'contents'=>'write'},job['permissions'])
      assert_includes job['env']['CARGO_REGISTRY_TOKEN'],'secrets.CARGO_REGISTRY_TOKEN'
      run = commands(job)
      assert_operator run.index('publish-core.sh'), :<, run.index('publish-frontend.sh')
    end
    assert_equal other['concurrency'], jobs['publish-crates']['concurrency']
    targets = JSON.parse(jobs['build']['with']['targets']).map { |row| row['target'] }
    smoke = jobs['archive-smoke']['strategy']['matrix']['include']
    assert_equal targets.sort, smoke.map { |row| row['target'] }.sort
    assert_includes commands(jobs['archive-smoke']), 'verify-archives.mjs'
    assert_includes commands(jobs['archive-smoke']), 'smoke.mjs'
    assert_includes commands(jobs['publish-npm']), 'verify-archives.mjs'
    assert_includes commands(jobs['publish-npm']), 'stage-npm.mjs'
    assert_includes commands(jobs['publish-npm']), 'publish-npm.mjs'
    assert_equal %w[alpine macos ubuntu windows], jobs['npm-smoke']['strategy']['matrix']['include'].map { |row| row['host'] }.sort
    assert_includes commands(jobs['npm-smoke']), 'node:24-alpine'
    assert_includes commands(jobs['promote-npm']), 'npm dist-tag add'
  end
  def test_frontend_publication_skips_existing_versions_and_fails_closed_on_registry_errors
    script = File.expand_path('../publish-frontend.sh', __dir__)
    Dir.mktmpdir do |dir|
      cargo = File.join(dir, 'cargo')
      curl = File.join(dir, 'curl')
      File.write(cargo, "#!/bin/sh\nprintf '%s' \"$*\" > \"$UPLOAD\"\n")
      File.write(curl, "#!/bin/sh\nprintf '%s' \"$STUB_STATUS\"\n")
      File.chmod(0755, cargo, curl)
      upload = File.join(dir, 'upload')
      env = {'PATH'=>dir + ':' + ENV.fetch('PATH'), 'UPLOAD'=>upload}
      ['200', '503', '404'].each do |status|
        out, result = Open3.capture2e(env.merge('STUB_STATUS'=>status),'bash',script,'crapts','0.1.0')
        assert_equal(status != '503',result.success?,out)
        assert_equal(status == '404',File.exist?(upload),out)
      end
      assert_equal 'publish --locked -p crapts', File.read(upload)
    end
  end
  def test_reusable_build_and_ci_contract
    w = workflow('_build')
    assert_equal %w[binary features package targets],w.fetch(true).fetch('workflow_call')['inputs'].keys.sort
    run = commands(w.fetch('jobs').fetch('build'))
    %w[--locked --release --features --target .tar.gz .zip].each { |token| assert_includes run, token }
    ci = workflow('ci')['jobs']
    assert_equal %w[macos-latest ubuntu-latest windows-latest], ci['npm-wrapper']['strategy']['matrix']['os'].sort
    assert_includes ci['npm-wrapper']['if'], 'needs.changes.outputs.npm'
    assert_includes commands(ci['npm-wrapper']), 'CRAPTS_BINARY'
    assert_includes commands(ci['npm-wrapper']), 'node --test'
    [ci['release-scripts'], workflow('action')['jobs']['scripts']].each do |job|
      line = commands(job).lines.find { |l| l.include?('./actionlint -ignore SC2016') }
      %w[_build release release-crapts ci action demo].each { |file| assert_includes line, ".github/workflows/#{file}.yml" }
    end
  end
end
