#nullable enable

using System;
using System.Buffers;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using System.IO;

namespace ForgeFFI;

/// <summary>
/// ForgeFFI NetIF（网卡管理）C# 中间层。
/// 
/// 目标：
/// - 自动按平台加载对应动态库（Windows/Linux/macOS）
/// - 调用 tool_netif_list_json / tool_netif_apply_json
/// - 自动回收 Rust 侧返回的内存（tool_free）
/// - 提供“按网卡名（如 eth0/WLAN）操作”的便捷方法（内部先 List 再解析 if_index）
/// 
/// 注意：
/// - Linux 下若只构建了 staticlib（.a），C# 无法 P/Invoke；需要构建 cdylib（.so）。
/// - 该封装仅负责“调用与内存安全”，权限/平台能力由底层实现决定。
/// </summary>
public sealed class ForgeFfiNetIf : IDisposable
{
    /// <summary>
    /// 通过环境变量指定动态库路径（可选）。
    /// - FORGEFFI_NETIF_LIB：优先使用的完整路径或库名
    /// </summary>
    public const string EnvLibPath = "FORGEFFI_NETIF_LIB";

    private const int ErrorCodeInvalidArgument = 1;

    private readonly IntPtr _lib;
    private readonly tool_netif_list_json_fn _list;
    private readonly tool_netif_apply_json_fn _apply;
    private readonly tool_free_fn _free;

    private bool _disposed;

    private ForgeFfiNetIf(IntPtr lib, tool_netif_list_json_fn list, tool_netif_apply_json_fn apply, tool_free_fn free)
    {
        _lib = lib;
        _list = list;
        _apply = apply;
        _free = free;
    }

    /// <summary>
    /// 自动加载动态库并初始化函数指针。
    /// 
    /// 加载顺序：
    /// 1) 环境变量 FORGEFFI_NETIF_LIB
    /// 2) 默认候选库名（按平台）
    /// 
    /// 你可以把 .dll/.so/.dylib 放到：
    /// - 进程当前目录
    /// - 系统动态库搜索路径（PATH/LD_LIBRARY_PATH/DYLD_LIBRARY_PATH）
    /// </summary>
    public static ForgeFfiNetIf LoadDefault()
    {
        var env = Environment.GetEnvironmentVariable(EnvLibPath);
        if (!string.IsNullOrWhiteSpace(env))
        {
            return LoadFromCandidates(new[] { env! });
        }

        return LoadFromCandidates(GetDefaultCandidates());
    }

    /// <summary>
    /// 使用给定候选库名/路径加载。
    /// </summary>
    public static ForgeFfiNetIf LoadFromCandidates(string[] candidates)
    {
        if (candidates is null || candidates.Length == 0)
        {
            throw new ArgumentException("candidates 不能为空", nameof(candidates));
        }

        var baseDir = AppContext.BaseDirectory;
        var cwd = Environment.CurrentDirectory;
        var env = Environment.GetEnvironmentVariable(EnvLibPath);

        IntPtr lib = IntPtr.Zero;
        Exception? last = null;

        foreach (var candidate in candidates)
        {
            if (string.IsNullOrWhiteSpace(candidate))
            {
                continue;
            }

            foreach (var path in ExpandCandidate(candidate, baseDir, cwd))
            {
                try
                {
                    if (NativeLibrary.TryLoad(path, out lib) && lib != IntPtr.Zero)
                    {
                        goto Loaded;
                    }
                }
                catch (Exception ex)
                {
                    last = ex;
                }
            }
        }

    Loaded:
        if (lib == IntPtr.Zero)
        {
            var detail =
                "未能加载 ForgeFFI 动态库。\n" +
                $"- 平台: {RuntimeInformation.OSDescription}\n" +
                $"- 进程架构: {RuntimeInformation.ProcessArchitecture}\n" +
                $"- AppContext.BaseDirectory: {baseDir}\n" +
                $"- 当前目录: {cwd}\n" +
                $"- 环境变量 {EnvLibPath}: {(string.IsNullOrWhiteSpace(env) ? "(未设置)" : env)}\n" +
                $"- 原始候选: {string.Join(", ", candidates)}\n" +
                "\n排查建议：\n" +
                "1) 把 libforgeffi_net_ffi.so 放到程序输出目录（与 .exe 同目录）\n" +
                "2) 或设置环境变量 FORGEFFI_NETIF_LIB 为 .so 的绝对路径\n" +
                "3) 确认 .so 架构匹配当前进程（aarch64/x86_64 不可混用）\n" +
                "4) 在 Linux 上用 ldd 检查依赖是否缺失：ldd libforgeffi_net_ffi.so\n";

            throw new DllNotFoundException(detail, last);
        }

        try
        {
            var listPtr = NativeLibrary.GetExport(lib, "tool_netif_list_json");
            var applyPtr = NativeLibrary.GetExport(lib, "tool_netif_apply_json");
            var freePtr = NativeLibrary.GetExport(lib, "tool_free");

            var list = Marshal.GetDelegateForFunctionPointer<tool_netif_list_json_fn>(listPtr);
            var apply = Marshal.GetDelegateForFunctionPointer<tool_netif_apply_json_fn>(applyPtr);
            var free = Marshal.GetDelegateForFunctionPointer<tool_free_fn>(freePtr);

            return new ForgeFfiNetIf(lib, list, apply, free);
        }
        catch
        {
            NativeLibrary.Free(lib);
            throw;
        }
    }

    /// <summary>
    /// 列出全部网卡（返回 JSON 字符串）。
    /// </summary>
    public NetifCallResult ListJson()
    {
        EnsureNotDisposed();
        IntPtr outPtr = IntPtr.Zero;
        nuint outLen = 0;
        var rc = _list(out outPtr, out outLen);
        var json = ReadAndFreeUtf8(outPtr, outLen);
        return new NetifCallResult(rc, json);
    }

    /// <summary>
    /// 发送原始请求 JSON（UTF-8）并返回响应 JSON。
    /// </summary>
    public NetifCallResult ApplyJson(string requestJson)
    {
        if (requestJson is null)
        {
            throw new ArgumentNullException(nameof(requestJson));
        }
        EnsureNotDisposed();

        var reqBytes = Encoding.UTF8.GetBytes(requestJson);
        unsafe
        {
            fixed (byte* pReq = reqBytes)
            {
                IntPtr outPtr = IntPtr.Zero;
                nuint outLen = 0;
                var rc = _apply(pReq, (nuint)reqBytes.Length, out outPtr, out outLen);
                var json = ReadAndFreeUtf8(outPtr, outLen);
                return new NetifCallResult(rc, json);
            }
        }
    }

    /// <summary>
    /// 添加 IP（if_index 定位）。
    /// </summary>
    public NetifCallResult AddIp(uint ifIndex, string ip, byte prefixLen)
    {
        var req = $"{{\"abi\":1,\"target\":{{\"if_index\":{ifIndex}}},\"ops\":[{{\"op\":\"add_ip\",\"ip\":{JsonString(ip)},\"prefix_len\":{prefixLen}}}]}}";
        return ApplyJson(req);
    }

    /// <summary>
    /// 删除 IP（if_index 定位）。
    /// </summary>
    public NetifCallResult DelIp(uint ifIndex, string ip, byte prefixLen)
    {
        var req = $"{{\"abi\":1,\"target\":{{\"if_index\":{ifIndex}}},\"ops\":[{{\"op\":\"del_ip\",\"ip\":{JsonString(ip)},\"prefix_len\":{prefixLen}}}]}}";
        return ApplyJson(req);
    }

    /// <summary>
    /// 设置 IPv4 DHCP 开/关（if_index 定位）。
    /// </summary>
    public NetifCallResult SetIpv4Dhcp(uint ifIndex, bool enable)
    {
        var req = $"{{\"abi\":1,\"target\":{{\"if_index\":{ifIndex}}},\"ops\":[{{\"op\":\"set_ipv4_dhcp\",\"enable\":{(enable ? "true" : "false")} }}]}}";
        return ApplyJson(req);
    }

    /// <summary>
    /// 设置静态 IPv4（if_index 定位）。
    /// 
    /// 参数：
    /// - ip/prefixLen：例如 10.80.158.234 + 23
    /// - gateway：可选；为空则不设置默认网关
    /// </summary>
    public NetifCallResult SetIpv4Static(uint ifIndex, string ip, byte prefixLen, string? gateway = null)
    {
        var gwPart = string.IsNullOrWhiteSpace(gateway) ? "" : $",\"gateway\":{JsonString(gateway!)}";
        var req = $"{{\"abi\":1,\"target\":{{\"if_index\":{ifIndex}}},\"ops\":[{{\"op\":\"set_ipv4_static\",\"ip\":{JsonString(ip)},\"prefix_len\":{prefixLen}{gwPart}}}]}}";
        return ApplyJson(req);
    }

    /// <summary>
    /// 通过网卡名解析 if_index（内部调用 ListJson 并解析 items）。
    /// 
    /// 说明：
    /// - name 匹配的是 list JSON 里的 items[].name（例如 eth0 / wlan0 / WLAN）。
    /// - 匹配规则：优先大小写敏感完全匹配，找不到则尝试忽略大小写匹配。
    /// </summary>
    public bool TryResolveIfIndexByName(string name, out uint ifIndex, out string? errorMessage)
    {
        ifIndex = 0;
        errorMessage = null;

        if (string.IsNullOrWhiteSpace(name))
        {
            errorMessage = "网卡名不能为空";
            return false;
        }

        var list = ListJson();
        if (!list.IsOk)
        {
            errorMessage = $"ListJson 失败，rc={list.Code}";
            return false;
        }

        try
        {
            using var doc = JsonDocument.Parse(list.Json);
            if (!doc.RootElement.TryGetProperty("items", out var items) || items.ValueKind != JsonValueKind.Array)
            {
                errorMessage = "list JSON 缺少 items 数组";
                return false;
            }

            uint? exact = null;
            uint? insensitive = null;

            foreach (var it in items.EnumerateArray())
            {
                if (!it.TryGetProperty("name", out var n) || n.ValueKind != JsonValueKind.String)
                {
                    continue;
                }
                if (!it.TryGetProperty("if_index", out var idx) || idx.ValueKind != JsonValueKind.Number)
                {
                    continue;
                }

                var itemName = n.GetString() ?? "";
                if (itemName.Length == 0)
                {
                    continue;
                }

                uint v;
                try
                {
                    v = idx.GetUInt32();
                }
                catch
                {
                    continue;
                }

                if (itemName == name)
                {
                    exact = v;
                    break;
                }
                if (insensitive is null && string.Equals(itemName, name, StringComparison.OrdinalIgnoreCase))
                {
                    insensitive = v;
                }
            }

            var resolved = exact ?? insensitive;
            if (resolved is null)
            {
                errorMessage = $"未找到网卡: {name}";
                return false;
            }

            ifIndex = resolved.Value;
            return true;
        }
        catch (Exception ex)
        {
            errorMessage = $"解析 list JSON 失败: {ex.Message}";
            return false;
        }
    }

    /// <summary>
    /// 添加 IP（按网卡名定位）。
    /// </summary>
    public NetifCallResult AddIpByName(string ifName, string ip, byte prefixLen)
    {
        return WithIfIndex(ifName, idx => AddIp(idx, ip, prefixLen));
    }

    /// <summary>
    /// 删除 IP（按网卡名定位）。
    /// </summary>
    public NetifCallResult DelIpByName(string ifName, string ip, byte prefixLen)
    {
        return WithIfIndex(ifName, idx => DelIp(idx, ip, prefixLen));
    }

    /// <summary>
    /// 设置 IPv4 DHCP 开/关（按网卡名定位）。
    /// </summary>
    public NetifCallResult SetIpv4DhcpByName(string ifName, bool enable)
    {
        return WithIfIndex(ifName, idx => SetIpv4Dhcp(idx, enable));
    }

    /// <summary>
    /// 设置静态 IPv4（按网卡名定位）。
    /// </summary>
    public NetifCallResult SetIpv4StaticByName(string ifName, string ip, byte prefixLen, string? gateway = null)
    {
        return WithIfIndex(ifName, idx => SetIpv4Static(idx, ip, prefixLen, gateway));
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _disposed = true;
        if (_lib != IntPtr.Zero)
        {
            NativeLibrary.Free(_lib);
        }
        GC.SuppressFinalize(this);
    }

    private NetifCallResult WithIfIndex(string ifName, Func<uint, NetifCallResult> action)
    {
        if (action is null)
        {
            throw new ArgumentNullException(nameof(action));
        }
        if (!TryResolveIfIndexByName(ifName, out var ifIndex, out var err))
        {
            return BuildInvalidArgumentResult(err ?? "解析网卡失败");
        }
        return action(ifIndex);
    }

    private static NetifCallResult BuildInvalidArgumentResult(string message)
    {
        var json = $"{{\"abi\":1,\"ok\":false,\"error\":{{\"code\":\"InvalidArgument\",\"message\":{JsonString(message)} }} }}";
        return new NetifCallResult(ErrorCodeInvalidArgument, json);
    }

    private void EnsureNotDisposed()
    {
        if (_disposed)
        {
            throw new ObjectDisposedException(nameof(ForgeFfiNetIf));
        }
    }

    private string ReadAndFreeUtf8(IntPtr ptr, nuint len)
    {
        if (ptr == IntPtr.Zero || len == 0)
        {
            return "";
        }

        int n;
        try
        {
            n = checked((int)len);
        }
        catch
        {
            _free(ptr, len);
            throw;
        }

        var rented = ArrayPool<byte>.Shared.Rent(n);
        try
        {
            Marshal.Copy(ptr, rented, 0, n);
            return Encoding.UTF8.GetString(rented, 0, n);
        }
        finally
        {
            ArrayPool<byte>.Shared.Return(rented);
            _free(ptr, len);
        }
    }

    private static string[] GetDefaultCandidates()
    {
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return new[] { "forgeffi_net_ffi.dll", "forgeffi_ffi.dll" };
        }
        if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
        {
            return new[] { "libforgeffi_net_ffi.dylib", "libforgeffi_ffi.dylib" };
        }
        return new[] { "libforgeffi_net_ffi.so", "libforgeffi_ffi.so" };
    }

    private static string[] ExpandCandidate(string candidate, string baseDir, string cwd)
    {
        var list = new string[3];

        list[0] = candidate;

        if (Path.IsPathRooted(candidate))
        {
            list[1] = candidate;
            list[2] = candidate;
            return list;
        }

        list[1] = Path.Combine(baseDir, candidate);
        list[2] = Path.Combine(cwd, candidate);
        return list;
    }

    private static string JsonString(string s)
    {
        var sb = new StringBuilder(s.Length + 2);
        sb.Append('"');
        foreach (var ch in s)
        {
            switch (ch)
            {
                case '"':
                    sb.Append("\\\"");
                    break;
                case '\\':
                    sb.Append("\\\\");
                    break;
                case '\b':
                    sb.Append("\\b");
                    break;
                case '\f':
                    sb.Append("\\f");
                    break;
                case '\n':
                    sb.Append("\\n");
                    break;
                case '\r':
                    sb.Append("\\r");
                    break;
                case '\t':
                    sb.Append("\\t");
                    break;
                default:
                    if (ch < 0x20)
                    {
                        sb.Append("\\u");
                        sb.Append(((int)ch).ToString("x4"));
                    }
                    else
                    {
                        sb.Append(ch);
                    }
                    break;
            }
        }
        sb.Append('"');
        return sb.ToString();
    }

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int tool_netif_list_json_fn(out IntPtr outPtr, out nuint outLen);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private unsafe delegate int tool_netif_apply_json_fn(byte* reqPtr, nuint reqLen, out IntPtr outPtr, out nuint outLen);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void tool_free_fn(IntPtr ptr, nuint len);
}

/// <summary>
/// 一次调用的返回结果。
/// </summary>
public readonly record struct NetifCallResult(int Code, string Json)
{
    /// <summary>
    /// Code==0 通常表示成功；非 0 时 Json 里仍会返回错误详情（ForgeFfiError）。
    /// </summary>
    public bool IsOk => Code == 0;
}

