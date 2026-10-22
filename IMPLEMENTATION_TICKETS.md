# Fluent Bit Pub/Sub Rust Plugin 実装チケット

このファイルは、`fluent-bit-pubsub-rs` の publish 機能拡張バックログです。
対象は「Fluent Bit Output プラグインとして Pub/Sub へ publish する機能」に限定します。

## P0 の再評価

| チケット | 判断 | 既定動作への影響 |
|----------|------|------------------|
| TKT-001 バッチ設定 | v0.1.1 で実装済み | 未指定は SDK 既定値のまま。従来から設定していた利用者は、v0.1.1 以降その値が有効になる点に注意。 |
| TKT-002 固定 attributes | 任意設定として実装 | 未指定は属性なし。指定時だけメタデータ・転送量が増える。本文は変わらない。 |
| TKT-003 ordering / resume | P2 に下げ、要件確定まで保留 | 有効化するとスループット・リージョン・再送順序の設計が必要。 |
| TKT-004 Endpoint | 任意設定として実装 | 未指定は従来どおり。指定時は送信先、遅延、リージョン障害時の挙動が変わる。 |

Avro / Protocol Buffers のスキーマ対応（旧 TKT-008）は利用予定がないため、対象から除外します。

## 優先度ガイド

- `P0`: すぐ着手。効果が大きく、実装リスクが低い。
- `P1`: 次点。運用性を大きく改善。
- `P2`: 重要だが重い。設計を固めてから着手。
- `P3`: 価値はあるが後回し可能。

## チケット一覧（優先順）

### TKT-001 (P0) バッチ設定をRust版へ導入
- 状態: v0.1.1 で実装済み。README に SDK 既定値も明記。
- 目的: スループットとレイテンシを設定で調整可能にする。
- 変更内容:
  - Fluent Bit設定キーを追加: `DelayThreshold`, `ByteThreshold`, `CountThreshold`
  - `Publisher::builder(...).set_delay_threshold(...).set_byte_threshold(...).set_message_count_threshold(...)` を適用
  - READMEに設定項目と既定値を追記
- 完了条件:
  - 設定値を読み取り、Pub/Sub publisherへ反映できる
  - 無指定時はライブラリ既定値（100件/1MB/10ms）で動作
  - 不正値（負数、非数値）のバリデーション方針が明示されている
- 受け入れテスト:
  - 単体: 設定パースの正常/異常系
  - 結合: E2E のオプションケースで CountThreshold=1 を指定して publish 成功を確認。閾値別の RPC 頻度・性能測定は未実施。
- 依存: なし
- 見積: S

### TKT-002 (P0) message attributes 対応
- 状態: 固定属性を実装。レコードからの動的抽出は対象外。
- 目的: 下流でのフィルタ/分類に使えるメタデータを付与する。
- 変更内容:
  - 設定キー追加: `Attributes`（`{"service":"api","environment":"production"}` 形式）
  - カンマや `=` を値に含めても曖昧にならないよう JSON を採用
  - `Message::set_attributes(...)` でpublish
  - 文字数・件数上限（Pub/Sub制約）に対する検証/ログ
- 完了条件:
  - 固定属性を全メッセージへ付与できる
  - 固定属性の制約違反は初期化時にエラーとし、送信開始前に検出する
- 受け入れテスト:
  - 単体: 属性パーサ
  - 結合: 属性付きpublishで subscriber 側に属性が見える
- 依存: なし
- 見積: S

### TKT-003 (P2・要設計) ordering key + resume publish 対応
- 状態: 要件確定まで保留。単純な設定追加と自動 resume だけでは実装しない。
- 先に決めること:
  - 順序保証の単位（全体、サービス、タスク、業務キー）と、複数 ECS タスク間の送信順序の扱い
  - 同一キーは同じリージョンへ送る。キーごとの上限 1 MB/s と遅延増加を許容できるか
  - Fluent Bit はチャンク単位で再送するため、一部成功・タイムアウト・後続チャンクとの順序をどう扱うか
  - 永続エラー時、キーを停止させるか、未送信分を順番に再送するか、欠落を許容して再開するか
  - Pub/Sub の配信順序と、BigQuery での検索結果の行順序は別であること（必要ならイベント時刻・連番と ORDER BY を用いる）
- 参考: [ordering](https://docs.cloud.google.com/pubsub/docs/ordering)、[ordering の再送](https://docs.cloud.google.com/pubsub/docs/retry-requests#retry_requests_with_ordering_keys)
- 目的: 同一キー内の順序保証を有効活用する。
- 変更内容:
  - 設定キー追加: `OrderingKey`（固定値）または `OrderingKeyField`（レコード由来）
  - `Message::set_ordering_key(...)` を適用
  - `PublishError::OrderingKeyPaused` または非リトライ系エラー時に `publisher.resume_publish(key)` の方針を実装
- 完了条件:
  - ordering key付きpublishができる
  - paused時にresume手段が実装され、挙動がREADMEに明記される
- 受け入れテスト:
  - 単体: key抽出
  - 結合: 意図的エラー後にresumeで再publish可能
- 依存: なし
- 見積: M

### TKT-004 (P0) Endpoint指定（locational endpoint含む）
- 状態: 任意設定 `Endpoint` を実装。未指定時のルーティング・認証は維持。
- 目的: ordered publish のリージョン固定や閉域網要件に対応する。
- 変更内容:
  - 設定キー追加: `Endpoint`（例: `https://us-east1-pubsub.googleapis.com`）
  - `Publisher::builder(...).with_endpoint(...)` を適用
  - READMEに利用例・注意点（ordering時は単一リージョン推奨）を追記
- 完了条件:
  - endpoint未指定時は既定エンドポイント
  - endpoint指定時に接続先が切り替わる
- 受け入れテスト:
  - 単体: 設定反映
  - 結合: E2E で明示 Endpoint を実際の emulator に向け、フォールバックのホストを無効にして優先順位を確認
  - 実クラウドのリージョン・閉域網は運用環境側で検証する
- 依存: なし
- 見積: S

### TKT-005 (P1) Retry/Backoff/Throttler設定の外部化
- 目的: 過負荷・一時障害時のpublish成功率と安定性を改善する。
- 変更内容:
  - 設定キー追加:
    - `RetryMaxAttempts`
    - `BackoffInitialMs`, `BackoffMaxMs`, `BackoffScaling`
    - `RetryThrottler`（`adaptive`/`circuit_breaker`）
  - `with_retry_policy(...)`, `with_backoff_policy(...)`, `with_retry_throttler(...)` を適用
- 完了条件:
  - デフォルト互換を維持しつつ、上書き可能
  - 設定が無効値のとき初期化失敗で明確にエラー
- 受け入れテスト:
  - 単体: 設定→ビルダー反映
  - 結合: 疑似的な失敗注入でリトライ挙動を確認
- 依存: TKT-004（builder経路整理の都合で推奨）
- 見積: M

### TKT-006 (P1) gRPC並行度（subchannel）設定
- 目的: 高負荷時のhead-of-line blockingを緩和する。
- 変更内容:
  - 設定キー追加: `GrpcSubchannelCount`
  - `with_grpc_subchannel_count(...)` を適用
  - 推奨値をREADMEへ追記（CPU/負荷依存）
- 完了条件:
  - 並行度を設定で変更可能
  - デフォルト動作は現状互換
- 受け入れテスト:
  - 単体: 設定反映
  - 負荷: 高並列publish時のスループット比較
- 依存: TKT-005（同時でも可）
- 見積: S

### TKT-007 (P1) Publisher側フロー制御（プラグイン内実装）
- 目的: 一時的スパイクでメモリ使用量が過大化するのを防ぐ。
- 変更内容:
  - 設定キー追加:
    - `MaxInFlightMessages`
    - `MaxInFlightBytes`
    - `FlowControlBehavior`（`block`/`drop`/`retry`）
  - `tokio::Semaphore` 等で `publish()` 投入数を制御
  - 制限到達時のログ・メトリクス追加
- 完了条件:
  - 上限超過時の挙動が設定通りになる
  - 無制限設定で現状互換
- 受け入れテスト:
  - 単体: in-flightカウンタ
  - 負荷: バースト投入時にメモリ上限を超えにくいこと
- 依存: TKT-001, TKT-005, TKT-006
- 見積: M-L

### TKT-009 (P3) Import Topicサポートの明文化と検証
- 目的: 対応可否の不明点をなくし、利用者の誤解を防ぐ。
- 変更内容:
  - READMEに「import topicへも通常publish可能」を明記
  - サンプル設定を追記
  - 簡易検証手順を追加
- 完了条件:
  - ドキュメントのみで利用判断可能
- 受け入れテスト:
  - 手動確認: import topic向けpublish例の再現
- 依存: なし
- 見積: XS

### TKT-010 (P3) Publish圧縮可否の技術調査
- 目的: 実装コストに見合うかを判断可能にする。
- 変更内容:
  - 依存クレートでrequest compression APIの有無を調査
  - 代替案（ネットワーク層圧縮、バッチ最適化）を比較
  - 採否判断メモを残す
- 完了条件:
  - 「やる/やらない」を根拠付きで決定可能
- 受け入れテスト:
  - N/A（調査チケット）
- 依存: なし
- 見積: S

## 実装フェーズ提案

### Phase 1（対応済み・検証対象）
- TKT-001: v0.1.1 で実装済み
- TKT-002: 固定 attributes のみ
- TKT-004: 任意 Endpoint のみ

### Phase 2（運用性強化）
- TKT-005
- TKT-006
- TKT-007

### Phase 3（高度機能）
- TKT-003: ordering の要件を合意してから
- TKT-009
- TKT-010

## Definition of Done（共通）

- `cargo test` が通る
- 既存設定との後方互換がある
- READMEに新規設定・既定値・注意点・サンプルが追記される
- エラーを `retryable / fatal` に分類し、理由がログで追える
