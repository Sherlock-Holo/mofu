# Rust

## 模块交互

模块之间只通过 trait 或纯数据 DTO 交互

## 代码质量约束

代码必须使用 Rust Edition 2024 风格来编写。

代码标准必须达到"明显没有问题"：结构清楚、边界清楚、错误路径可读、类型表达意图，避免依靠隐含约定或偶然可用的状态。实现时优先让数据模型、trait 和消息类型表达不变量，少用散落的字符串和临时状态。

不得滥用 `#[must_use]`。

不允许新增任何 `#[allow(dead_code)]`。

不得新增任何本项目代码引起的 `cargo clippy` warn，除非是依赖本身导致的。

### 依赖版本

`Cargo.toml` 中新增或修改的依赖版本必须写完整的 `x.y.z`，不得只写 `x.y`。例如使用 `prost = "0.14.4"`，不要写
`prost = "0.14"`。

### 导入路径

优先直接 `use` 类型本身，使用时写短名，不要写 `crate::xxx::yyy::Z` 这类完整路径。对于函数，可以使用函数所在层级的 mod，例如 `call_me()` 可以使用 `yyy::call_me()`，但不要写 `crate::xxx::yyy::call_me()`。

- 类型在当前文件**不会冲突**：直接导入并使用短名，例如 `use foo::Bar` 后写 `Bar`。
- 类型在当前文件**会冲突**：用 `as` 起别名绕开，例如 `use foo::Message as FooMessage`。
- 没有直观别名可选时：保留一层 parent，例如 `foo::Message`；仍不要写 `crate::...` 完整路径。

### 错误命名

所有错误变量优先使用 `err` 作为变量名，除非出现冲突无法解决，可以改名为 `xxx_err`。

### tracing 字段写法

使用 `info!` / `warn!` / `error!` 时，能写成 `%xxx` 或 `?xxx` 的字段不要写 `xxx = %xxx` / `xxx = ?xxx`。例如写 `error!(%err, "open failed")`，不要写 `error!(error = %err, "open failed")`。

### Clone 写法

对已实现 `Clone` 的类型，直接调用 `.clone()`，不要写 `Arc::clone(&value)` 等 `Type::clone(&x)` 形式。

### 测试模块导入

`#[cfg(test)] mod tests` 必须放在源文件末尾，位于所有生产代码（类型定义、`impl`、常量等）之后。

`#[cfg(test)] mod tests` 中优先写 `use super::*`，复用父模块本地定义的符号（类型、函数、`pub(crate)` 项等）。父模块的 `use crate::...` 不会被子模块继承；tests 里缺少什么，就在 `mod tests` 内直接补 `use crate::...` 或 `use std::...`。不要为测试在父模块写 `pub(crate) use` 或 `#[cfg(test)] pub(crate) use`。

### 函数归属

逻辑应优先写成主要操作对象的 method（有 `self`）或 associated function（无 `self`），不要写成模块级裸函数，也不要挂在无关类型上；外部 crate 类型无法直接 `impl` 时，先用本地 newtype 包装。serde 的 `deserialize_with` / `serialize_with` / `skip_serializing_if` 回调除外。

### 类型选择

能用 enum 表达有限集合的地方，不用 `String` 或 `int` 裸写法。外部规范中的固定标签集合（如 OpenCollection Spec 中的 value type 字段）应映射为 enum。

### 异步 trait（AFIT）

定义异步 trait 时，trait 侧使用返回 `impl Future` 的同步方法签名；具体类型实现时直接写 `async fn`：

```rust
pub trait Example {
    fn load(&self, id: &Id) -> impl Future<Output = Result<Item, Error>> + Send;
}

impl Example for FileExample {
    async fn load(&self, id: &Id) -> Result<Item, Error> {
        // ...
    }
}
```

不要在实现里再包一层 `fn load(...) -> impl Future { self.load_inner(...) }` 转发。需要复用的逻辑应提取为私有 `async fn` 辅助方法，由 trait 实现直接调用。

返回 `dyn Trait` 或需要对象安全的异步方法时，可继续使用 `async-trait`。

## 注释和内联文档要求

public trait、核心数据结构、非显然的枚举变体补充简短 /// 文档。
复杂逻辑前使用少量注释说明原因，例如变量解析顺序、数据构造、兼容策略等。

## 代码格式化

所有代码必须通过 `cargo fmt` 格式化。
