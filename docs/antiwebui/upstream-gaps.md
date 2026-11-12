# AntiWebUI の不具合と足りないところ

移行の間（2026-11-12 から）に見つけた AntiWebUI と md3e の不具合、足りない機能、知らないと困る振る舞いと、
Hiphile の側で暫定に入れた手をまとめる。上流が直したら、その手を外す。

AntiWebUI はリビジョン `25e82a1` に固定している。上流のパスはそのリポジトリの `src/` からの相対パスで、
`dNN` はその `docs/design/NN-*.md` を指す。計画を立てたときに読んで分かったことは
[UI を AntiWebUI に移す](migration.md)の「足りないところ」と「ここでビルドして分かったこと」にあり、ここでは
今の手と一緒にもう一度挙げる。

## 不具合

### フォーカスのあるノードが消えても、フォーカスを失ったと知らされない

`dyn_view` が作り直した部品や、ページを替えて消えた入力欄は `on_focus_change(false)` を受けない。

- パスワードの表示を切り替えるボタンは作り直されると、自分でフォーカスを取り直す
  （`next/account/src/form.rs`、`toggle_focused`）。
- 設定の数値の欄は、打ちかけのまま消えるときに、打った値を確定する（`next/settings/src/number_field.rs`、
  `is_typed_into`）。
- 設定のポップアップを Escape や支援技術で閉じるときは、閉じる前に面のフォーカスを外して確定させる
  （`next/settings/src/popup.rs`）。

### `NodeRef::focus` は描画を求めない

`NodeRef::focus` は `pending_focus` を置くだけで（`anti-webui-core/src/view.rs:2230`）、次の描画まで効かない。
ほかに描くものが無いと、フォーカスは動かないままになる。

- `next_ui::dialog_parts` の `focus` は、`pending_focus` を置いたうえで `mark_paint` を呼ぶ。ダイアログと
  サインインの最初のフォーカスはこれを通す。

### 絶対配置の子が、親の輪と状態レイヤーの上に描かれる

`absolute` の子は重なりの順の一覧から描かれ、入れ物自身の `outline` と `Ink` の状態レイヤーより後になる。
md3e の `image` も絶対配置なので、カードの縁の輪はカバーが縁に接するところで隠れる。

- 輪は、最後の子に置いた `absolute().inset(0)` と `pointer_events_none()` の層に描く（`next/ui/src/card_grid.rs`、
  `next/hiphile/src/server_menu.rs` のアバター、`next/player/src/bar.rs` の `cover_toggle`）。
  `pointer_events_none` を付けないと、カードの中のリンクへのクリックをこの層が取る。

### `effect` が自分の読むシグナルを書くと、走り直しが落ちる

`memo` を通して読んでいても同じで、`memo` は古い値のまま残る。

- 書く先は `untrack` で読むか、書くのをハンドラに移す（[コーディングのガイド](coding-guide.md)の「リアクティブの
  基本」）。

### モーダルでないものを閉じると、フォーカスがどこにも無くなる

モーダルなポップアップは閉じるとフォーカスを返すが、モーダルでないものは返さない。md3e のメニューは
モーダルでなく（`anti-webui-md3e/src/components/menu.rs:157`）、開くとフォーカスを取り、閉じても元の場所に
返さない。フォーカスのある行が消えたときと、フォーカスを取らないところを押したときも
同じになる。フォーカスが無いとキーはウィンドウの根にしか届かず、画面のキーが効かない。

- シェルは `next_ui::keymap::keeps_focus` で根を包む。フォーカスが無い間に来たキーは、最後にフォーカスの
  あった `handles_keys` の要素に回す。それも無ければ、画面の `Screen::home` に回す。
- 歌詞のオーバーレイは、開く前にフォーカスのあったところを `next_ui::keymap::FocusBefore` で覚え、閉じたら
  そこに返す（`next/player/src/lyrics_view.rs`）。

### `UiSender` は、チャンネルのオーナーが消えたあとも送られたものを積み続ける

`anti-webui-core/src/channel.rs:67-73`。

- コアへの橋渡し（`next_host::follow` など）は `on_cleanup` でタスクを止め、オーナーが消えたあとは送らない。

### macOS で 1 つのスレッドにウィンドウが 2 つあると、再描画が再帰しうる

macOS の `Session` に `request_redraw` と `redraw_pending` が無く、runner のメソッドが自分を呼ぶ
（`anti-webui-app/src/runner.rs:688-693`）。読んだだけで、実行はしていない。

- ウィンドウはメインの 1 つだけにしている。ミニプレーヤーを戻すときは、`spawn_window` で専用のスレッドに
  開く。

### 小数のリテラルが、将来の lint でエラーになりうる

スタイルの setter が `impl Into<f32>` を取るので、`.p(16.0)` のリテラルの型が決まらず、
`float_literal_f32_fallback` に当たる（`anti-webui-style/src/styled.rs:3-15`）。AntiWebUI 自身の widgets と
md3e にも 287 か所ある。

- Hiphile では `16.0_f32` と書く。この lint がハードエラーになると、上流が直すまで AntiWebUI がビルドできない。

## 足りない機能

### フォーカスとキー

- **フォーカスとウィンドウの根を読む公開の口が無い。** `next_ui::keymap::focus` は、文書の
  `state.focused` と `pending_focus` を直接読み書きし、根の `key_down` を差し替える
  （[非公開の内部に触れているところ](#非公開の内部に触れているところ)）。
- **フォーカスが見えるところまでスクロールしない。** スクロールするのはテキストのキャレットだけ。
  設定のページは Tab と Shift-Tab のあと、1 フレーム待ってフォーカスを見えるところまでスクロールする
  （`next/settings/src/focus.rs`）。
- **`outlined_text_field` から中の入力欄に届かない。** `next_ui::dialog_parts` の `first_input` が、部品の
  ノードの中から入力欄を探す。
- **ダイアログは中身より後にフォーカスを取る。** 中身から先にフォーカスを取ることはできない。
  `focused_once_laid_out` と `focused_field` は、最初のレイアウトのあとにフォーカスを移す。
- **キーマップもキーの文脈も無い。** `on_key_down` は、フォーカスのあるノードから上に伝わる。
  `MenuItem::shortcut` は文字を出すだけ（`anti-webui-widgets/src/menu.rs:54`）。`next_ui::keymap` に、
  文脈、有効かどうか、メニューに出すキーの文字を持たせた。

### 文字

- **省略記号も行数の制限も無い。** はみ出しは切り、右端を薄くして続きがあることを示す
  （`next_ui::row_text::faded_end`）。「…」は出ない。
- **数字の幅を揃える指定（`tnum`）が無い。** 長さと番号は決まった幅の列に右揃えで置く（`DURATION_LANE`、
  `next_ui::song_row`）。
- **ロケールを環境変数からしか読まない。** Finder から開いた `.app` では日本語の字形にならない
  （`anti-webui-core/src/text/mod.rs:74-79`）。`next-platform` が `main` の最初に、システムのロケールから
  `LANG` を入れる。ロケールの言語が日中韓でなければ、AppleLanguages の中の最初の日中韓の言語を使う
  （`en_JP` の地域で日本語を使うときのため、`next/platform/src/locale.rs`）。

### md3e の部品

- **ボタンに色を渡せない。** 取り返しのつかない Delete は、テキストボタンの上に文字の色だけを `error` で
  重ねる（`next_ui::dialog_parts::irreversible_button`）。ポインタが乗ったときの状態レイヤーは primary の
  まま。
- **`TopAppBar` に戻ると進むを置けない。** ナビゲーションのボタンが 1 つに決め打ちで、見出しは 60 px の
  位置にある。画面の見出しは `next_ui::app_bar`。
- **`dropdown_field` は選択肢が決まった一覧で、押したときにしか開かない。** フォーカスはテキストの入力欄に
  入り、その名前は中に出るラベルだけ。設定は自前の `choice_field` を使う
  （`next/settings/src/choice_field.rs`）。
- **アイコンボタンのアイコンは、作るときに決まる。** 状態でアイコンが変わる再生のボタンなどは、自前の
  `icon_control` にした（`next/player/src/control.rs`）。
- **`linear_progress` は、新しい値まで 1 秒近くかけて動く。** シークバーの平らな線は自前で描く
  （`next/player/src/position.rs` の `flat_line`）。
- **波線のスライダーは、ポインタが乗っているか掴んでいる間しか波が流れない**
  （`anti-webui-md3e/src/components/slider.rs:162-175`）。`slider_with` の振る舞いに、再生中は
  `wavy_progress_eased`、一時停止中は平らな線の見た目を載せる。
- **`image` は下に面を敷き、薄く現れる。** 飛んでいるカバーのように見えているものには、
  `next_ui::cover::picture_at_once` を使う。
- **`carousel` にはキーもフォーカスも無く、縦のホイールを取ってページを止める。** Home とアーティストの
  ページは `next_ui::card_grid::card_row` を使う。
- **`dialog` の幅は最大 560 px。** 設定のポップアップは、widgets の `modal_with` に md3e の面を載せる
  （`next/settings/src/popup.rs`）。
- **`menu` は要素にしか付けられない**（`MenuStyle::anchor(NodeRef)`）。ポインタの位置に出すメニューは、
  押された点に `portal` で大きさのある見えない要素を置き、そこに付ける（`next_ui::context_menu`）。
- **スナックバーは 1 つずつ順に出る。** アクションのあるものは閉じるまで残り、後のものを止める。元に戻す
  スナックバーには `lasting` で時間を付ける。
- **色を選ぶ部品が無い。** 設定の色は、widgets の `radio_group_with` の振る舞いに自前の見本の見た目を
  載せる（`next/settings/src/swatches.rs`）。
- **寸法を外に出さない。** segmented list の角（`next/settings/src/group.rs` の `OUTER` と `INNER`）と、
  検索のバーの寸法（`next/search/src/results.rs` の `ROOM`）は、md3e の値を写して持つ。

### レイアウトと描画

- **子を形で切り抜けない**（d15:303）。切り抜けるのは `overflow_hidden` の箱の角の丸みだけで、
  `MaterialShape` の形にはならない。カードのカバーは、カバー自身の上の角を丸める
  （`next/ui/src/card_grid.rs`）。
- **角の丸みは、箱の短い辺の半分までに切り詰められる**（`anti-webui-wgpu/src/renderer.rs:382-383`、
  `anti-webui-wgpu/src/shader.wgsl:44`）。CSS のように隣り合う角の和で縮めないので、40×20 の箱の上の角を
  20 にしても半円にならない。設定の色の見本は、丸めた `overflow_hidden` の箱の中に四角を並べて切り抜く
  （`next/settings/src/swatches.rs`）。
- **`virtual_list` は行の高さが 1 種類で、キー操作が無い。** `table` は 1 行しか選べない。
  `next_ui::row_list` が、曲についていく選択、Shift と ⌘ での選択、キーでの移動、ドラッグでの並べ替えを持つ。
  見出しの固定は無い。
- **`split_row` は比率で分け、px の最小と最大が無い。** 一覧と詳細は `next_ui::list_detail` で分ける。
- **要素をまたぐ動きと、スクロールのアニメーションが無い。** 歌詞のカバーの飛行は、バーのカバーの
  `logical_rect` を出発点にして `animated` のばねで動かす。歌詞が行を追う動きは `request_frame` と
  `scroll_to` で作る。
- **ホイールは、ポインタの下の要素の祖先にしか届かない。** 歌詞の「Sync」ボタンは、受けたホイールを
  行のスクロール領域に渡す（`next/player/src/lyrics_view/lines.rs`）。

### ウィンドウとプラットフォーム

`next-platform` が objc2 で AppKit を直接呼ぶ。AntiWebUI が開いた `NSWindow` は、タイトルで探す。

- **メニューバーが固定。** 「隠す」「ほかを隠す」「すべてを表示」「終了」しか無い
  （`anti-webui-macos/src/app.rs:64-92`）。`next-platform` が `NSApp` のメインメニューを置き換える。メニューに
  あるキーはメニューが先に取るので、画面のキーとは重ねない（`next/hiphile/src/menu_bar/tests.rs` が確かめる）。
  メニューを開いている間は、AntiWebUI のループが止まる。
- **ウィンドウの最小サイズ、枠の保存、前に出すことができない**（`anti-webui-core/src/window.rs:82-102`）。
  `next-platform` の `MainWindow` が設定する。
- **クライアント側の装飾では、信号機ボタンが隠れる**（`anti-webui-macos/src/state.rs:77-89`）。
  `next-platform` が見せ直し、ボタンはレールの上の余白に重なる。ドラッグは `start_window_move()`
  （`next_ui::window::moves_window`）。
- **システムの外観に従わない。** `next-platform` が `NSApp` の `effectiveAppearance` を KVO で見る。
- **動きを減らす設定を読まない**（d13:294-299）。`next-platform` が読み、Hiphile の動きはそれに従う。
  md3e の部品の動きは止まらない。
- **ウィンドウハンドルに届かない**（`anti-webui-macos/src/lib.rs:429-445`）。メディアコントロールは
  ハンドル無しでつなぐ（`next/hiphile/src/reach.rs`）。macOS ではハンドルが要らない。
- **⌘Q と Dock の「終了」は、最初のウィンドウを閉じる要求になる。** `App::run` が戻ったあと、
  `Hiphile::shutdown()` を呼ぶ。

### 部品の寿命

- **子のオーナーを作る関数が `doc(hidden)`**（`anti-webui-core/src/reactive.rs:428-430`）。カバーの取得は、
  公開の `create_root` で専用のオーナーを作り、終わったら捨てる（`next/ui/src/cover.rs`）。

### アクセシビリティ

- **役割と名前はテスト用の木にしかならず、スクリーンリーダーには届かない**（d34:115）。受け入れて、
  名前と役割はテストのために付け続ける。

## 知っておくこと

不具合ではないが、知らないと迷う振る舞い。

- **キーで動かしたフォーカスは、キーで来たフォーカスとして見える。** キーのハンドラの中で
  `NodeRef::focus` を呼ぶと、移った先は `on_focus_visible(true)` を受ける。ポインタで押したあとは受けない
  （`anti-webui-core/src/ui.rs:1670`）。`next_ui::selection::KeysShown` はこれに頼る。
- **md3e の部品が中身から取った名前の頭に、アイコンの字形が入る。** レールの項目は `"\u{e9b2} Home"`。
  `next_ui::testing::find` と `press` はそれを除いて比べる。
- **md3e のメニューの項目の名前には、後ろに出るキーも入る**（`"Remove ⌫"`）。
- **md3e と prelude で名前が重なる**（`button`、`menu`、`switch`、`dialog`、prelude の `follow` と
  `next_host::follow`）。md3e のものは名前を挙げて import する。
- **`NodeRef::focus` を `with_doc` の中で呼ぶと、借用の二重で panic する。** 文書を借りるのをやめてから呼ぶ。
- **`Ui` を落としても、スレッドの文書は残る。** 新しい文書になるのは `Ui::close` だけで、そこでノードの
  id が振り直される。新しいウィンドウの根が古い根と同じ id になりうる。`keeps_focus` は、根の id ではなく
  差し替えたハンドラそのもので、自分が包んだかを見分ける。
- **閉じる動きの間、ダイアログはモーダルのまま。** アクセシビリティの木にも閉じたダイアログが残り、
  テストの `has_text` と `find` に見える。閉じたことを待つときは `frames` で動きを進め、`is_modal_open()` を
  見る。
- **ばねとタイマーは `Instant::now()` から進む**（`anti-webui-core/src/animation.rs:102`、`:484`）。テストで
  動きの途中を確かめるときは、動きを始める前に取った時刻からフレームを描く
  （`next/player/src/lyrics_view/tests.rs` の `frames_from`）。
- **headless の描画は、画像があると 1 秒ぶんフレームを進めてから描く**
  （`anti-webui-app/src/headless.rs:44-52`）。動きの途中の絵は、`shots` が `Gpu` と `Renderer` で自分で描く。

## 非公開の内部に触れているところ

AntiWebUI が文書と semver の約束から外している `anti_webui::core::tree` を使っている。リビジョンを上げる
ときは、ここから確かめる。

| 場所 | 触れているもの | なぜ |
| --- | --- | --- |
| `next/ui/src/keymap/focus.rs` | `state.focused`、`pending_focus`、根の `key_down` | フォーカスと根を読む口が無い |
| `next/ui/src/dialog_parts.rs` | `pending_focus`、`mark_paint`、ノードの子 | `NodeRef::focus` が描画を求めず、入力欄に届かない |
| `next/settings/src/focus.rs` | `state.focused`、`pending_scroll`、`dirty.paint` | フォーカスを見えるところまでスクロールしない |
| `next/ui/src/testing.rs` | ポップアップの一覧、`ancestors`、`computed.outline_width` | モーダルの下のものを押さない確認と、輪が出ているかの確認。アクセシビリティの木の id はノードの id を `slotmap` の ffi の値にしたものなので、`slotmap` で戻す |
