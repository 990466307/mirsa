# DSH 接入 MIRSA

先按 [MCP 接入说明](README.md) 在 Linux / WSL 安装服务。Windows DSH 通过 `wsl.exe` 调用，无须在 Windows 重新编译 MIRSA。

## 配置

将 [dsh.cordis.yml](dsh.cordis.yml) 的 `insert` 项合并到 **DSH 当前实际使用的 profile 的 patch 配置**，保留其他项目，同名项目不要重复添加。仓库附带的是本机示例，使用前将发行版 `Ubuntu-D`、用户 `wentao` 和仓库路径改成你的实际值。

对应启动命令为：

```powershell
wsl.exe -d Ubuntu-D -u wentao --exec /bin/bash /home/wentao/mirsa/scripts/start-mcp.sh
```

桌面端应修改当前 desktop profile；不要另外启动一套 web profile 来验证桌面会话。配置加载后，工具列表应出现：

```text
mcp__mirsa__mirsa_read_lines
mcp__mirsa__mirsa_variables
mcp__mirsa__mirsa_query
```

若工具列表未刷新，重新连接 MCP 或新建会话。

## 让 DSH 试用

可以直接将下面这段话发给 DSH：

> 使用 MIRSA 检查这段代码的切片下标边界。先确定语句行号和变量，再查询性质；分别说明已证明、模型内确定违反或无法判断，并给出实际工具结果。
>
> ```rust
> pub fn example(s: &[u8], i: usize) -> u8 {
>     if i < s.len() {
>         unsafe { *s.get_unchecked(i) }
>     } else { 0 }
> }
> ```

DSH 应调用 `mirsa_query`，传入同一份 `code`、`line: 3` 和 `dsl: "$i < len($s) at 'mirsa_line_entry"`，得到 `conclusion.proved=true`。继续查询 `facts($s) at 'mirsa_line_entry` 可查看相关抽象信息及符号关系。

文件分析流程是 `mirsa_read_lines` → `mirsa_variables` → `mirsa_query`。所有参数与结果含义见 [MCP 接口](README.md)；不需要用户手写源码标注。

`false` 不能直接解释为漏洞；单个性质成立也不代表整个 unsafe 调用安全。代码缺少依赖、超时或绑定失败时，应报告未完成分析。
