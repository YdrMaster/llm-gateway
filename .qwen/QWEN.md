# 项目备忘录 - RustLLM Gateway

## Markdown 写作规范

- 遵循 markdownlint 规则
- 注意 MD060 表格列样式（表格列对齐）
- 使用严格等宽字体排版，中文（全角）字符显示宽度为 2 个英文（半角）字符，Markdown 表格按此标准对齐
- 存放图、树或者程序输出等非代码的代码块标注 plaintext
- 不要在标题之前加 --- 分隔线，例如：

  ```markdown, 禁止这样写
  some content.

  ---

  ## any title
  ```

## Rust struct 可见性设计原则

1. **组织数据的 struct（POD 类型）** - 所有字段都应为 `pub`
   - 例如：`RoutingPayload`, `NodeMetrics`
   - 作为数据载体，不包含复杂逻辑

2. **实现功能的 struct** - 所有字段都应为非 `pub`
   - 例如：`InputNode`, `VirtualNode`, `OutputNode`, `RoutingGraph`
   - 包含复杂逻辑或对字段状态有要求的方法
   - 通过方法访问，避免外部代码意外修改字段状态导致异常

## Rust 开发规范即风格

1. **使用 Rust skills** - 开发 Rust 代码时应使用 rust-router 等相关技能
2. **文本语言** 所有**注释和文档**使用中文，所有超过一行的函数必须有 `///` 文档，所有会输出到控制台或日志的文本使用英文
3. **文档提醒** 模块级文档使用 `//!`，代码结构文档使用 `///`，普通注释使用 `//`
4. **内联格式串** 格式串应该尽量使用内联形式，例如 `"{x}"`，而不是 `"{}", x`
5. **代码简洁** 使用简洁的代码，省略所有可推导的类型标注，省略所有不必要的省略号
6. **验证** - 依次执行 `cargo fmt`、`cargo check`、`cargo clippy` 和 `cargo test`，其中 clippy 必须解决所有非 unused 类的警告，除非在 TDD 阶段，否则 test 必须全部通过

## Git 提交规范

**重要：** 在任何模式下（包括 YOLO），每次提交 git 之前必须由人类 review

- 提交前执行 `git status` 和 `git diff --staged` 查看变更
- 等待人类确认提交信息和变更内容
- 获得批准后再执行 `git commit`

## TDD 开发流程

对于每个功能模块：

```plaintext
1. 文档准备（协议调研/技术调研）
       ↓
2. 设计接口（定义 Trait/Struct）
       ↓
3. 编写测试（此时编译失败或测试失败）
       ↓
4. 实现功能（测试通过）
       ↓
5. 重构优化
```
