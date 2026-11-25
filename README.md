# Shardline

Shardline 是面向大型、频繁变动文件的内容寻址存储服务器。它在多次上传之间复用已校验的内容，并通过持久化元数据记录文件版本。实际存储复用率取决于协议和对象布局。它适合存储模型检查点、数据集、容器镜像层、游戏资源、构建产物和 CI 缓存。

Shardline 可部署在自有基础设施上，提供常用客户端协议。上传和下载全程采用流式处理，服务端内存用量有明确上限。

## 与上游的关系

本仓库源自 [STEXS Technologies 的 Shardline](https://github.com/STEXS-Technologies/shardline)，由本仓库维护者独立维护和发布。原始代码的版权归属和双许可继续保留。上游提交和本仓库的修改说明见 [NOTICE](NOTICE)。

本仓库对工作树和提交历史做了大幅整理，包括重组工作树、压缩历史分支的合并提交，以及归并早期版本标签之间的开发提交。双方现在有各自的维护历史，常规上游分支合并已不适用。需要引入上游修复时，按具体提交审查、移植并在独立实例验证。

README 已在本仓库翻译成中文，并更新了本地启动和分发说明。

## 主要特性

- **跨版本去重**：内容完全相同的上传可以复用已有数据。部分改动能节省多少空间取决于上传协议。[存储基准与实测限制](docs/BENCHMARKS.md#operational-efficiency-measurements)
- **流式处理**：上传、下载、预览、范围读取和后端处理均限制内存用量。
- **断点续传**：Git LFS、OCI、S3、Xet 等协议支持在传输中断后继续上传。
- **统一存储核心**：Git LFS、OCI、Bazel、S3、Hugging Face 客户端、Xet 和原生 CLI 共用内容寻址存储。
- **本地或云存储**：小规模部署可使用本地文件系统。生产部署可使用兼容 S3 的对象存储和 PostgreSQL 元数据。
- **运维与诊断**：服务端提供请求限制、并发限制、取消、完整性校验、垃圾回收、修复工具、指标和故障演练。

## 支持的协议和工具

| 协议或工具 | 典型用途 |
| --- | --- |
| Git LFS | Git 仓库中的大文件 |
| Hugging Face Hub API | 模型、数据集和仓库版本 |
| OCI Distribution | 容器镜像和制品 |
| S3 兼容 API | 应用、数据分析和对象存储工具 |
| Bazel HTTP 缓存 | 远程构建产物和操作缓存 |
| Xet | 仓库文件的分块存储和重建 |
| 原生 CLI | 管理、维护、迁移和性能测试 |

各协议共享权限控制、去重、存储限额、生命周期元数据和可观测性。不同协议的编码方式仍可能产生需要分别存储的对象。

## 升级到 1.13.0

已有上传数据可直接读取。启用新写入之前，由数据库所有者执行元数据迁移，并按[升级流程](docs/ROLLING_UPGRADE.md)切换实例。请暂停 Xet 原生上传，直到所有读取原生分块的实例完成升级。新版本上传会省略冗余的独立分块副本，旧版读取实例可能依赖这些副本。

新版本的 Hub Git 对象标识和 S3 标签也受升级指南所述的路由与回滚限制。恢复写入前，请同时检查就绪状态和旧副本是否已退出服务。

版本变化见[更新日志](CHANGELOG.md)。已验证的客户端流程及现有限制见[兼容性状态](docs/COMPATIBILITY_STATUS.md)。

## 在本地启动

开发用 Compose 配置会启动 Shardline，以 PostgreSQL 保存元数据，以 MinIO 保存对象。此配置适合本地验证。生产环境的配置见[部署指南](docs/DEPLOYMENT.md)。

运行环境需要 Docker Engine、Compose 插件和至少 4 GiB 可用内存。Compose 使用主机网络，主机的 5432、9001、18080 和 19000 端口须空闲。Docker Desktop 4.34 及以上版本需先开启[主机网络](https://docs.docker.com/engine/network/drivers/host/#docker-desktop)。

在本仓库的本地检出根目录运行以下命令。

```bash
docker compose up --build
```

服务监听 `http://127.0.0.1:18080`。首次启动会构建镜像、执行数据库迁移、创建开发用存储桶，随后等待后端就绪并启动 API。

在另一个终端检查进程状态和后端就绪状态。

```bash
curl -fsS http://127.0.0.1:18080/healthz
curl -fsS http://127.0.0.1:18080/readyz
```

Compose 配置包含开发凭据。共享环境或对外开放服务前，请更换凭据。

停止服务时保留数据卷。

```bash
docker compose down
```

只有在需要重新初始化本地开发环境时，才执行下面这条会删除数据卷的命令。

```bash
docker compose down --volumes
```

## 第一次发送认证请求

本地 Compose 配置使用 HMAC 认证。在运行中的容器内签发一个权限范围限定到具体仓库的令牌。

```bash
TOKEN="$(docker compose exec -T shardline \
  shardline admin token \
  --issuer local \
  --subject local-operator \
  --scope write \
  --provider generic \
  --owner example \
  --repo assets \
  --revision main \
  --key-env SHARDLINE_TOKEN_SIGNING_KEY)"

curl -fsS \
  -H "Authorization: Bearer ${TOKEN}" \
  http://127.0.0.1:18080/v1/stats
```

生产环境应使用文件挂载签名密钥，或接入外部身份提供方。签发客户端凭据前，请阅读[客户端配置](docs/CLIENT_CONFIGURATION.md)和[认证指南](docs/AUTHENTICATION.md)。

## Hugging Face 和数据集

Shardline 提供兼容 Hub 的 API，支持仓库版本、文件上传与下载、范围读取和有资源限额的数据集预览。兼容客户端可以直接连接 Shardline。

```bash
export HF_ENDPOINT=http://127.0.0.1:18080
export HF_TOKEN="${TOKEN}"

hf upload example/assets ./weights
hf download example/assets model.safetensors --revision main
```

数据集预览支持 CSV、JSONL 和 Parquet。服务端按需流式读取当前页面所需的数据范围，并限制扫描字节数、行数、返回结果大小和执行时间。

## 用 DuckDB 分析 Parquet

需要执行完整 SQL 时，可以让独立的 DuckDB 进程作为 S3 客户端访问 Shardline。查询的计算和资源消耗由 DuckDB 进程承担，支持谓词下推、选择列、聚合、联接和 Parquet 导出。

```sql
INSTALL httpfs;
LOAD httpfs;

CREATE SECRET shardline (
  TYPE S3,
  KEY_ID 'REPOSITORY_SCOPED_TOKEN',
  SECRET 'unused',
  ENDPOINT 'shardline.example:443',
  REGION 'us-east-1',
  URL_STYLE 'path',
  USE_SSL true
);

SELECT label, count(*) AS rows
FROM read_parquet('s3://example.assets/data/train/*.parquet')
WHERE label = 'positive'
GROUP BY label;
```

Hub 也提供用于授权数据集预览的原生查询接口，支持获取表结构、选择列、校验过滤条件、有限聚合、稳定排序和分页。每次请求绑定到已授权、不可变的仓库版本和文件标识。服务端可以观测扫描量、结果大小、并发、执行时间和取消状态，并对执行资源设限。

[S3 设置、查询接口、使用场景和基准测试](docs/DUCKDB.md)

## 生产部署

小规模部署可以运行一个使用本地存储的 Shardline 进程。需要持久化的生产部署通常包括以下组件。

- Shardline API 和传输服务
- PostgreSQL 兼容的元数据存储
- S3 兼容的对象存储
- 可选的 Redis 文件重建缓存
- 边缘 TLS 终止和身份提供方

API 和传输服务可分别扩容，共享同一套元数据和对象存储，并执行同样的内容校验、权限校验和资源限制。

环境变量、TLS、密钥、存储适配器、角色拆分、Kubernetes 和扩容见[部署指南](docs/DEPLOYMENT.md)。备份、恢复顺序、垃圾回收、修复和故障应对见[运维指南](docs/OPERATIONS.md)。

## 命令行工具

`shardline` 可启动服务并执行日常运维任务。

| 命令 | 用途 |
| --- | --- |
| `shardline serve` | 启动服务 |
| `shardline config` | 校验生效的配置 |
| `shardline db` | 执行或检查 PostgreSQL 迁移 |
| `shardline health` | 探测服务健康状态 |
| `shardline fsck` | 验证对象和元数据的完整性 |
| `shardline index` | 重建可变索引 |
| `shardline repair` | 修复生命周期元数据和 Webhook 状态 |
| `shardline backup` | 导出恢复所需的制品 |
| `shardline storage` | 在存储适配器之间迁移对象 |
| `shardline gc` | 执行垃圾回收或安装定时任务 |
| `shardline hold` | 管理数据保留锁 |
| `shardline bench` | 运行性能基准测试 |

用 `shardline completion` 生成 Shell 补全脚本，用 `shardline manpage` 生成手册页。运行 `shardline help <command>` 查看指定命令的配置和安全说明。

## 安全和数据保护

- 在各协议入口检查身份和仓库权限范围。
- 支持通过挂载文件提供密钥，避免将密钥放入进程参数。
- 校验对象键、标识符、读取范围和序列化元数据。
- 对象按内容寻址发布，重复发布同一内容是幂等的。
- 垃圾回收先隔离候选对象，再执行删除。过程可以安全中断和恢复。
- 指标和错误信息避免暴露凭据、SQL、内部路径和行数据。

[安全约束](docs/SECURITY_AND_INVARIANTS.md) · [兼容性状态](docs/COMPATIBILITY_STATUS.md) · [故障演练](docs/CHAOS_ENGINEERING.md)

## 从源码构建与测试

工作区使用 Rust 2024。安装 Rust 和项目开发工具后运行以下命令。

```bash
cargo check --workspace
cargo make ci
```

CI 检查格式、Clippy、依赖策略、工作区测试、发布打包和数据库迁移同步。依赖 Docker 的测试和端到端测试见[贡献指南](CONTRIBUTING.md)。

以下命令用于单独运行 Hub API 测试、服务故障演练和 DuckDB 基准测试。

```bash
cargo nextest run -p shardline-hub-api
cargo nextest run -p shardline-server --test fault_drills
cargo make shardline-bench-duckdb
```

## 文档

从[文档索引](docs/README.md)进入，也可以按任务查阅下列指南。

- [快速入门](docs/GETTING_STARTED.md)
- [部署](docs/DEPLOYMENT.md)
- [客户端配置](docs/CLIENT_CONFIGURATION.md)
- [协议入口](docs/PROTOCOLS.md)
- [DuckDB 集成](docs/DUCKDB.md)
- [架构](docs/ARCHITECTURE.md)
- [运维](docs/OPERATIONS.md)
- [维护命令与结果](docs/MAINTENANCE_INTEGRATION_UX.md)
- [只读管理 API](docs/ADMIN_READ_API.md)
- [安全约束](docs/SECURITY_AND_INVARIANTS.md)
- [协议一致性](docs/PROTOCOL_CONFORMANCE.md)
- [协调发布](docs/RELEASE.md)

## 许可证

Shardline 沿用上游的双许可。使用、修改和分发 Shardline 代码时，可选择遵循 [MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE) 的条款。上游版权及来源记录见 [NOTICE](NOTICE)。

二进制发布包包含项目许可证、`NOTICE` 和第三方依赖声明 `THIRD_PARTY_NOTICES.txt`。容器镜像内的对应文件位于 `/usr/share/licenses/shardline/`。第三方依赖按各自许可分发，依赖声明列出确切版本及其源码获取地址。Debian 系统包的版权和许可文件位于镜像内的 `/usr/share/doc/`。

单独向 Cargo 注册表分发工作区 crate 时，也要在各源码包中附带适用的许可证和版权声明。独立注册表发布使用本仓库维护者控制的包名或注册表权限。
