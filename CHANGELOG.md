# Changelog

## [0.4.0](https://github.com/bearingsoftware/simple-markdown-editor/compare/v0.3.0...v0.4.0) (2026-11-11)


### Features

* **MDEDIT-139:** publish each release to winget ([628a9ce](https://github.com/bearingsoftware/simple-markdown-editor/commit/628a9ce29b6024b14780c9322586d399adc6bfbc))


### Bug Fixes

* **deps:** update rust crate rfd to 0.18 ([a40b537](https://github.com/bearingsoftware/simple-markdown-editor/commit/a40b537d5597083e8b5b798ac91cbd659bff8310))
* **deps:** update rust crate rfd to 0.18 ([da959df](https://github.com/bearingsoftware/simple-markdown-editor/commit/da959df036c10f5daa5374d8795b60f5f6a0cec2))
* **MDEDIT-139:** install a pinned, verified Komac instead of an action that fetches it unpinned ([ac8cac5](https://github.com/bearingsoftware/simple-markdown-editor/commit/ac8cac51ba4634bf158ab7e336f0ab8bc864c19b))
* **MDEDIT-139:** let winget match the installed app by its listed name ([21354a6](https://github.com/bearingsoftware/simple-markdown-editor/commit/21354a6d315cc3bad2d2d194a12bd03d764e35d6))
* **MDEDIT-139:** link the C runtime statically so the exe runs without the VC++ redistributable ([6cdfd98](https://github.com/bearingsoftware/simple-markdown-editor/commit/6cdfd9848b62235d77c86b23ecf60017d37dc026))
* **MDEDIT-202:** bound a fence by work rather than time ([2534c88](https://github.com/bearingsoftware/simple-markdown-editor/commit/2534c88e348452aa61532f020ef18663096f4dfb))
* **MDEDIT-202:** bound one fence's highlighting by a line cap and a time budget ([fcae536](https://github.com/bearingsoftware/simple-markdown-editor/commit/fcae5364a4cbcb1001d8fbe2fd7c6bed50b1d650))
* **MDEDIT-202:** charge every line a fixed cost as well as its length ([a0f0bc8](https://github.com/bearingsoftware/simple-markdown-editor/commit/a0f0bc8e4a97f832ce059d8bd53da359e7bccc51))
* **MDEDIT-202:** size the work budget from measured cost ([161a24a](https://github.com/bearingsoftware/simple-markdown-editor/commit/161a24a4ebf944c473024353871748998c62cd3f))


### Documentation

* **MDEDIT-139:** drop a doubled blank line ([61832a0](https://github.com/bearingsoftware/simple-markdown-editor/commit/61832a0688ecb1f396a83bae8f41e4ec552a3d39))
* **MDEDIT-202:** record the 4 MiB work budget and its measurements ([0eb2a4a](https://github.com/bearingsoftware/simple-markdown-editor/commit/0eb2a4a46d0f48199a03bbd8ff5c9e9ccb0f0a94))
* **MDEDIT-202:** record the line cap, the budget and their measurements ([b8e55bb](https://github.com/bearingsoftware/simple-markdown-editor/commit/b8e55bb2d32b5530d18d9d88d3d55737f7fbd2c6))
* **MDEDIT-202:** record the per-line charge and its measurements ([12a221c](https://github.com/bearingsoftware/simple-markdown-editor/commit/12a221c424a3fbe3a391b35b452005fccbad0d57))
* **MDEDIT-202:** record the work budget and the hostile-fence result ([a1e89bb](https://github.com/bearingsoftware/simple-markdown-editor/commit/a1e89bbcd1118042c9e6d2784712fa80fe61ca3a))
* **MDEDIT-202:** record the worst case measured on a quiet machine ([df01b54](https://github.com/bearingsoftware/simple-markdown-editor/commit/df01b541c3f97b7d033a72e241a85e0ff1f0674b))

## [0.3.0](https://github.com/bearingsoftware/simple-markdown-editor/compare/v0.2.0...v0.3.0) (2026-11-06)


### Features

* **MDEDIT-200:** ship third-party licence notices with the zip and installer ([2d5d102](https://github.com/bearingsoftware/simple-markdown-editor/commit/2d5d10205830ff737eedc594d40995b3a75f88ea))
* **MDEDIT-201:** build the syntax set at compile time, highlight TypeScript fences as JavaScript ([3fb45d4](https://github.com/bearingsoftware/simple-markdown-editor/commit/3fb45d4d9b2e79c8622b4e264ad71935749dbcfe))
* **MDEDIT-201:** vendor TOML, Dockerfile, Kotlin and PowerShell syntaxes ([231fc9a](https://github.com/bearingsoftware/simple-markdown-editor/commit/231fc9abe690b6d1014b9e1dd41f0d954b1cf938))


### Bug Fixes

* **MDEDIT-200:** blank line before the syntax notices ([a0fd5cc](https://github.com/bearingsoftware/simple-markdown-editor/commit/a0fd5cc6c6bcbd39bda2736df8dd3469ef7f1661))
* **MDEDIT-201:** allow the build script's panic on a bad vendored file ([95ef586](https://github.com/bearingsoftware/simple-markdown-editor/commit/95ef586eb703e500fb170ca0e46a764dbd52e67f))


### Documentation

* **MDEDIT-201:** record first-touch and hostile-line costs ([23394bc](https://github.com/bearingsoftware/simple-markdown-editor/commit/23394bc3439ddf9601aad97e9505304351a2cf78))
* **MDEDIT-201:** record the four fence languages and the build-time dump ([4338885](https://github.com/bearingsoftware/simple-markdown-editor/commit/4338885722f28f97170628db7e6aec970e54245e))

## [0.2.0](https://github.com/bearingsoftware/simple-markdown-editor/compare/v0.1.0...v0.2.0) (2026-11-05)


### Features

* **MDEDIT-190:** colour fenced code on a worker thread ([f27f01f](https://github.com/bearingsoftware/simple-markdown-editor/commit/f27f01f2e8a250918521392136c8d3cd79fecedf))
* **MDEDIT-190:** highlight fenced code by its info string ([8626cca](https://github.com/bearingsoftware/simple-markdown-editor/commit/8626ccaed435e36519325cde05e223d902ec6372))


### Bug Fixes

* **MDEDIT-190:** name the highlight job type for clippy ([d3d6c27](https://github.com/bearingsoftware/simple-markdown-editor/commit/d3d6c273045ac1a7e4e2ec1ef1071795f856ecc3))
* **MDEDIT-190:** review fixes for the highlight worker ([66ddf20](https://github.com/bearingsoftware/simple-markdown-editor/commit/66ddf20facfb1f9dab2af3bb161fc5567bc923ee))


### Documentation

* **MDEDIT-190:** point the syntect notes at the outcome ([91bc788](https://github.com/bearingsoftware/simple-markdown-editor/commit/91bc7883f89f07629344d1725cdda227c83a24a1))
* **MDEDIT-190:** record the worker, its numbers and the licence audit ([dee8a60](https://github.com/bearingsoftware/simple-markdown-editor/commit/dee8a60b22f003aa78a7f56dfa6f4597a1610e88))

## [0.1.0](https://github.com/bearingsoftware/simple-markdown-editor/compare/v0.1.0...v0.1.0) (2026-11-03)


### Features

* **MDEDIT-137:** a release builds a portable zip and a per-user installer ([d4bfa6a](https://github.com/bearingsoftware/simple-markdown-editor/commit/d4bfa6a6a9c10fceaab51fd92953f875533a36b8))
* **MDEDIT-140:** release-please versions and changelogs ([52bca13](https://github.com/bearingsoftware/simple-markdown-editor/commit/52bca139b1c319df4554267e56535a1829b2ca20))


### Bug Fixes

* **MDEDIT-137:** install Inno Setup 6.7.4 from its release, pinned by hash ([c5752ab](https://github.com/bearingsoftware/simple-markdown-editor/commit/c5752abc0b136a3c309f73e0406921df38a2ced4))
* **MDEDIT-137:** the manifest check fails unless asInvoker is declared ([a650e16](https://github.com/bearingsoftware/simple-markdown-editor/commit/a650e16e1425f79149880123dd2dba61109d65ec))
* **MDEDIT-140:** pin release-please-action to a commit ([688a696](https://github.com/bearingsoftware/simple-markdown-editor/commit/688a696d371038a2c51d6fdc5eadbbeb83c3bb12))
