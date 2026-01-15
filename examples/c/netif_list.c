#if defined(_WIN32)
#define _CRT_SECURE_NO_WARNINGS 1
#endif

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <stdarg.h>

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <dlfcn.h>
#endif

typedef int32_t (*tool_netif_list_json_fn)(uint8_t **out_ptr, size_t *out_len);
typedef int32_t (*tool_netif_apply_json_fn)(const uint8_t *req_ptr, size_t req_len, uint8_t **out_ptr, size_t *out_len);
typedef void (*tool_free_fn)(uint8_t *ptr, size_t len);

#if defined(FORGEFFI_STATIC)
int32_t tool_netif_list_json(uint8_t **out_ptr, size_t *out_len);
int32_t tool_netif_apply_json(const uint8_t *req_ptr, size_t req_len, uint8_t **out_ptr, size_t *out_len);
void tool_free(uint8_t *ptr, size_t len);
#endif

typedef struct {
    uint32_t if_index;
    char name[128];
    char ipv4[256];
    char ipv6[256];
} iface_row;

typedef struct {
    char *buf;
    size_t len;
    size_t cap;
} strbuf;

static const char *skip_ws(const char *p);
static const char *find_key(const char *obj, const char *key);
static const char *find_object_end(const char *p);
static int parse_u32_after_colon(const char *p, uint32_t *out);
static int parse_bool_after_colon(const char *p, int *out);
static int parse_optional_json_string(const char *p, char *out, size_t out_cap);
static size_t parse_ifaces(const char *json, iface_row *rows, size_t cap);

static int sb_reserve(strbuf *sb, size_t need_cap) {
    if (need_cap <= sb->cap) {
        return 1;
    }
    size_t new_cap = sb->cap ? sb->cap : 256;
    while (new_cap < need_cap) {
        new_cap = new_cap + (new_cap >> 1);
        if (new_cap < sb->cap) {
            return 0;
        }
    }
    char *p = (char *)realloc(sb->buf, new_cap);
    if (p == NULL) {
        return 0;
    }
    sb->buf = p;
    sb->cap = new_cap;
    return 1;
}

static int sb_append_n(strbuf *sb, const char *s, size_t n) {
    if (!sb_reserve(sb, sb->len + n + 1)) {
        return 0;
    }
    memcpy(sb->buf + sb->len, s, n);
    sb->len += n;
    sb->buf[sb->len] = '\0';
    return 1;
}

static int sb_append(strbuf *sb, const char *s) {
    return sb_append_n(sb, s, strlen(s));
}

static int sb_appendf(strbuf *sb, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    va_list ap2;
    va_copy(ap2, ap);
    int need = vsnprintf(NULL, 0, fmt, ap);
    va_end(ap);
    if (need < 0) {
        va_end(ap2);
        return 0;
    }
    if (!sb_reserve(sb, sb->len + (size_t)need + 1)) {
        va_end(ap2);
        return 0;
    }
    int wrote = vsnprintf(sb->buf + sb->len, sb->cap - sb->len, fmt, ap2);
    va_end(ap2);
    if (wrote != need) {
        return 0;
    }
    sb->len += (size_t)wrote;
    return 1;
}

static void sb_free(strbuf *sb) {
    if (sb->buf != NULL) {
        free(sb->buf);
        sb->buf = NULL;
    }
    sb->len = 0;
    sb->cap = 0;
}

static void *load_library(void) {
#if defined(FORGEFFI_STATIC)
    return NULL;
#else
#if defined(_WIN32)
    const char *candidates[] = {"forgeffi_net_ffi.dll", "forgeffi_ffi.dll"};
    for (size_t i = 0; i < (sizeof(candidates) / sizeof(candidates[0])); i++) {
        HMODULE h = LoadLibraryA(candidates[i]);
        if (h != NULL) {
            return (void *)h;
        }
    }
    return NULL;
#else
#if defined(__APPLE__)
    const char *candidates[] = {"libforgeffi_net_ffi.dylib", "libforgeffi_ffi.dylib"};
#else
    const char *candidates[] = {"libforgeffi_net_ffi.so", "libforgeffi_ffi.so"};
#endif
    for (size_t i = 0; i < (sizeof(candidates) / sizeof(candidates[0])); i++) {
        void *h = dlopen(candidates[i], RTLD_NOW | RTLD_LOCAL);
        if (h != NULL) {
            return h;
        }
    }
    return NULL;
#endif
#endif
}

static void *load_symbol(void *lib, const char *name) {
#if defined(FORGEFFI_STATIC)
    (void)lib;
    (void)name;
    return NULL;
#else
#if defined(_WIN32)
    return (void *)GetProcAddress((HMODULE)lib, name);
#else
    return dlsym(lib, name);
#endif
#endif
}

static void close_library(void *lib) {
#if defined(FORGEFFI_STATIC)
    (void)lib;
#else
#if defined(_WIN32)
    if (lib != NULL) {
        FreeLibrary((HMODULE)lib);
    }
#else
    if (lib != NULL) {
        dlclose(lib);
    }
#endif
#endif
}

static int read_line(char *buf, size_t cap) {
    if (cap == 0) {
        return 0;
    }
    if (fgets(buf, (int)cap, stdin) == NULL) {
        return 0;
    }
    size_t n = 0;
    while (buf[n] != '\0') {
        if (buf[n] == '\r' || buf[n] == '\n') {
            buf[n] = '\0';
            break;
        }
        n++;
    }
    return 1;
}

static int is_ipv6_literal(const char *s) {
    while (*s != '\0') {
        if (*s == ':') {
            return 1;
        }
        s++;
    }
    return 0;
}

static const char *error_code_to_zh(const char *code) {
    if (code == NULL) {
        return "未知错误";
    }
    if (strcmp(code, "Ok") == 0) {
        return "成功";
    }
    if (strcmp(code, "InvalidArgument") == 0) {
        return "参数错误";
    }
    if (strcmp(code, "NotFound") == 0) {
        return "未找到";
    }
    if (strcmp(code, "Unsupported") == 0) {
        return "不支持";
    }
    if (strcmp(code, "PermissionDenied") == 0) {
        return "权限不足";
    }
    if (strcmp(code, "SystemError") == 0) {
        return "系统错误";
    }
    return "未知错误";
}

static void read_optional_gateway(char *out, size_t out_cap) {
    if (out_cap == 0) {
        return;
    }
    out[0] = '\0';
    fprintf(stderr, "请输入网关（可留空）: ");
    char line[256];
    if (!read_line(line, sizeof(line))) {
        return;
    }
    if (line[0] == '\0') {
        return;
    }
    strncpy(out, line, out_cap - 1);
    out[out_cap - 1] = '\0';
}

static int print_apply_response_pretty(const char *json) {
    int ok = 0;
    const char *ok_p = find_key(json, "ok");
    if (ok_p != NULL) {
        (void)parse_bool_after_colon(ok_p, &ok);
    }

    fprintf(stdout, "\n---- 操作结果摘要 ----\n");
    fprintf(stdout, "总体: %s\n", ok ? "成功" : "失败");

    const char *results = strstr(json, "\"results\"");
    if (results == NULL) {
        return 1;
    }
    const char *p = strchr(results, '[');
    if (p == NULL) {
        return 1;
    }
    p++;

    while (*p != '\0') {
        p = skip_ws(p);
        if (*p == ']') {
            break;
        }
        if (*p != '{') {
            p++;
            continue;
        }
        const char *obj_start = p;
        const char *obj_end = find_object_end(obj_start);
        if (obj_end == NULL) {
            break;
        }

        uint32_t i = 0;
        int step_ok = 0;
        const char *i_p = find_key(obj_start, "i");
        if (i_p != NULL) {
            (void)parse_u32_after_colon(i_p, &i);
        }
        const char *sok_p = find_key(obj_start, "ok");
        if (sok_p != NULL) {
            (void)parse_bool_after_colon(sok_p, &step_ok);
        }

        if (step_ok) {
            fprintf(stdout, "- 第 %u 步: 成功\n", (unsigned)i);
        } else {
            char code[64];
            char msg[256];
            code[0] = '\0';
            msg[0] = '\0';
            const char *err_p = find_key(obj_start, "error");
            if (err_p != NULL) {
                err_p = skip_ws(err_p);
                if (*err_p == '{') {
                    const char *c_p = find_key(err_p, "code");
                    if (c_p != NULL) {
                        (void)parse_optional_json_string(c_p, code, sizeof(code));
                        if (code[0] == '\0') {
                            uint32_t code_num = 0;
                            if (parse_u32_after_colon(c_p, &code_num)) {
                                (void)snprintf(code, sizeof(code), "%u", (unsigned)code_num);
                            }
                        }
                    }
                    const char *m_p = find_key(err_p, "message");
                    if (m_p != NULL) {
                        (void)parse_optional_json_string(m_p, msg, sizeof(msg));
                    }
                }
            }
            if (code[0] != '\0') {
                fprintf(stdout, "- 第 %u 步: 失败（%s）: %s\n", (unsigned)i, error_code_to_zh(code), msg[0] ? msg : "(无详情)");
                if (strcmp(code, "PermissionDenied") == 0) {
                    fprintf(stdout, "  提示: Linux 上修改网卡通常需要 sudo/root 权限\n");
                }
                if (strcmp(code, "Unsupported") == 0) {
                    fprintf(stdout, "  提示: Linux 上 DHCP 配置依赖 NetworkManager/systemd-networkd，不在本接口范围\n");
                }
            } else {
                fprintf(stdout, "- 第 %u 步: 失败\n", (unsigned)i);
            }
        }

        p = obj_end + 1;
    }

    fprintf(stdout, "----------------------\n\n");
    return 1;
}

static int ask_if_index(const uint8_t *last_json, size_t last_len, uint32_t *out_if_index) {
    char line[256];
    if (last_json != NULL && last_len != 0) {
        iface_row rows[64];
        size_t n = parse_ifaces((const char *)last_json, rows, (sizeof(rows) / sizeof(rows[0])));
        if (n != 0) {
            fprintf(stdout, "可用网卡列表: \n");
            for (size_t i = 0; i < n; i++) {
                const char *name = rows[i].name[0] ? rows[i].name : "(无名称)";
                fprintf(stdout, "  [%zu] if_index=%u  名称=%s\n", i, (unsigned)rows[i].if_index, name);
            }
            fprintf(stderr, "请输入 if_index（或上面列表序号）: ");
        } else {
            fprintf(stderr, "请输入 if_index: ");
        }
    } else {
        fprintf(stderr, "请输入 if_index: ");
    }

    if (!read_line(line, sizeof(line))) {
        return 0;
    }

    if (last_json != NULL && last_len != 0) {
        iface_row rows[64];
        size_t n = parse_ifaces((const char *)last_json, rows, (sizeof(rows) / sizeof(rows[0])));
        if (n != 0) {
            char *end = NULL;
            unsigned long vv = strtoul(line, &end, 10);
            while (end != NULL && (*end == ' ' || *end == '\t')) {
                end++;
            }
            if (end != NULL && *end == '\0') {
                uint32_t v = (uint32_t)vv;
                if (v != 0 && v < n) {
                    *out_if_index = rows[v].if_index;
                    return 1;
                }
                for (size_t i = 0; i < n; i++) {
                    if (rows[i].if_index == v) {
                        *out_if_index = v;
                        return 1;
                    }
                }
            } else {
                for (size_t i = 0; i < n; i++) {
                    if (strcmp(rows[i].name, line) == 0) {
                        *out_if_index = rows[i].if_index;
                        return 1;
                    }
                }
            }
        }
    }

    return 0;
}

static size_t gather_ipv4_entries_for_iface(const char *json, uint32_t if_index, iface_row *tmp_row, char ips[][96], uint32_t *prefixes, size_t cap) {
    (void)tmp_row;
    const char *items = strstr(json, "\"items\"");
    if (items == NULL) {
        return 0;
    }
    const char *p = strchr(items, '[');
    if (p == NULL) {
        return 0;
    }
    p++;

    while (*p != '\0') {
        p = skip_ws(p);
        if (*p == ']') {
            break;
        }
        if (*p != '{') {
            p++;
            continue;
        }
        const char *obj_start = p;
        const char *obj_end = find_object_end(obj_start);
        if (obj_end == NULL) {
            break;
        }

        uint32_t idx = 0;
        const char *idx_p = find_key(obj_start, "if_index");
        if (idx_p != NULL) {
            (void)parse_u32_after_colon(idx_p, &idx);
        }

        if (idx == if_index) {
            const char *a = find_key(obj_start, "ipv4");
            if (a == NULL) {
                return 0;
            }
            a = skip_ws(a);
            if (*a != '[') {
                return 0;
            }
            a++;
            size_t n = 0;
            while (*a != '\0' && n < cap) {
                a = skip_ws(a);
                if (*a == ']') {
                    break;
                }
                if (*a != '{') {
                    a++;
                    continue;
                }
                const char *e_start = a;
                const char *e_end = find_object_end(e_start);
                if (e_end == NULL) {
                    break;
                }
                char ip[96];
                ip[0] = '\0';
                uint32_t pl = 0;
                const char *ip_p = find_key(e_start, "ip");
                if (ip_p != NULL) {
                    (void)parse_optional_json_string(ip_p, ip, sizeof(ip));
                }
                const char *pl_p = find_key(e_start, "prefix_len");
                if (pl_p != NULL) {
                    (void)parse_u32_after_colon(pl_p, &pl);
                }
                if (ip[0] != '\0') {
                    strncpy(ips[n], ip, 95);
                    ips[n][95] = '\0';
                    prefixes[n] = pl;
                    n++;
                }
                a = e_end + 1;
            }
            return n;
        }

        p = obj_end + 1;
    }

    return 0;
}

static const char *skip_ws(const char *p) {
    while (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n') {
        p++;
    }
    return p;
}

static int parse_bool_after_colon(const char *p, int *out) {
    p = skip_ws(p);
    if (strncmp(p, "true", 4) == 0) {
        *out = 1;
        return 1;
    }
    if (strncmp(p, "false", 5) == 0) {
        *out = 0;
        return 1;
    }
    return 0;
}

static const char *find_key(const char *obj, const char *key) {
    char pat[64];
    size_t klen = 0;
    while (key[klen] != '\0' && klen + 3 < sizeof(pat)) {
        klen++;
    }
    if (klen == 0 || klen + 3 >= sizeof(pat)) {
        return NULL;
    }
    pat[0] = '"';
    for (size_t i = 0; i < klen; i++) {
        pat[i + 1] = key[i];
    }
    pat[klen + 1] = '"';
    pat[klen + 2] = ':';
    pat[klen + 3] = '\0';
    const char *hit = strstr(obj, pat);
    if (hit == NULL) {
        return NULL;
    }
    return hit + (klen + 3);
}

static const char *find_object_end(const char *p) {
    if (p == NULL || *p != '{') {
        return NULL;
    }
    int depth = 0;
    int in_str = 0;
    int esc = 0;
    for (; *p != '\0'; p++) {
        char c = *p;
        if (in_str) {
            if (esc) {
                esc = 0;
                continue;
            }
            if (c == '\\') {
                esc = 1;
                continue;
            }
            if (c == '"') {
                in_str = 0;
                continue;
            }
            continue;
        }

        if (c == '"') {
            in_str = 1;
            continue;
        }
        if (c == '{') {
            depth++;
            continue;
        }
        if (c == '}') {
            depth--;
            if (depth == 0) {
                return p;
            }
            continue;
        }
    }
    return NULL;
}

static int parse_u32_after_colon(const char *p, uint32_t *out) {
    p = skip_ws(p);
    uint64_t v = 0;
    int any = 0;
    while (*p >= '0' && *p <= '9') {
        any = 1;
        v = v * 10u + (uint64_t)(*p - '0');
        if (v > 0xFFFFFFFFu) {
            return 0;
        }
        p++;
    }
    if (!any) {
        return 0;
    }
    *out = (uint32_t)v;
    return 1;
}

static int parse_json_string(const char *p, char *out, size_t out_cap) {
    p = skip_ws(p);
    if (*p != '"') {
        return 0;
    }
    p++;
    size_t o = 0;
    while (*p != '\0') {
        if (*p == '"') {
            if (o < out_cap) {
                out[o] = '\0';
            } else if (out_cap != 0) {
                out[out_cap - 1] = '\0';
            }
            return 1;
        }
        if (*p == '\\') {
            p++;
            if (*p == '\0') {
                return 0;
            }
            if (*p == '"' || *p == '\\' || *p == '/') {
                if (o + 1 < out_cap) {
                    out[o++] = *p;
                }
                p++;
                continue;
            }
            if (*p == 'b') {
                if (o + 1 < out_cap) {
                    out[o++] = '\b';
                }
                p++;
                continue;
            }
            if (*p == 'f') {
                if (o + 1 < out_cap) {
                    out[o++] = '\f';
                }
                p++;
                continue;
            }
            if (*p == 'n') {
                if (o + 1 < out_cap) {
                    out[o++] = '\n';
                }
                p++;
                continue;
            }
            if (*p == 'r') {
                if (o + 1 < out_cap) {
                    out[o++] = '\r';
                }
                p++;
                continue;
            }
            if (*p == 't') {
                if (o + 1 < out_cap) {
                    out[o++] = '\t';
                }
                p++;
                continue;
            }
            if (*p == 'u') {
                if (o + 1 < out_cap) {
                    out[o++] = '?';
                }
                p++;
                for (int i = 0; i < 4; i++) {
                    if ((*p >= '0' && *p <= '9') || (*p >= 'a' && *p <= 'f') || (*p >= 'A' && *p <= 'F')) {
                        p++;
                    } else {
                        return 0;
                    }
                }
                continue;
            }
            return 0;
        }
        if (o + 1 < out_cap) {
            out[o++] = *p;
        }
        p++;
    }
    return 0;
}

static int parse_optional_json_string(const char *p, char *out, size_t out_cap) {
    p = skip_ws(p);
    if (out_cap != 0) {
        out[0] = '\0';
    }
    if (strncmp(p, "null", 4) == 0) {
        return 1;
    }
    return parse_json_string(p, out, out_cap);
}

static void str_append(char *dst, size_t dst_cap, const char *src) {
    if (dst_cap == 0) {
        return;
    }
    size_t d = 0;
    while (d + 1 < dst_cap && dst[d] != '\0') {
        d++;
    }
    size_t s = 0;
    while (d + 1 < dst_cap && src[s] != '\0') {
        dst[d++] = src[s++];
    }
    dst[d] = '\0';
}

static void parse_ip_array(const char *obj, const char *key, char *out, size_t out_cap) {
    if (out_cap == 0) {
        return;
    }
    out[0] = '\0';
    const char *p = find_key(obj, key);
    if (p == NULL) {
        return;
    }
    p = skip_ws(p);
    if (*p != '[') {
        return;
    }
    p++;
    int first = 1;
    while (*p != '\0') {
        p = skip_ws(p);
        if (*p == ']') {
            return;
        }
        if (*p != '{') {
            p++;
            continue;
        }
        const char *obj_start = p;
        const char *obj_end = strchr(obj_start, '}');
        if (obj_end == NULL) {
            return;
        }
        char tmp_ip[96];
        tmp_ip[0] = '\0';
        uint32_t prefix = 0;
        const char *ip_p = find_key(obj_start, "ip");
        if (ip_p != NULL) {
            (void)parse_json_string(ip_p, tmp_ip, sizeof(tmp_ip));
        }
        const char *pl_p = find_key(obj_start, "prefix_len");
        if (pl_p != NULL) {
            (void)parse_u32_after_colon(pl_p, &prefix);
        }
        if (tmp_ip[0] != '\0') {
            char one[128];
            if (snprintf(one, sizeof(one), "%s/%u", tmp_ip, (unsigned)prefix) > 0) {
                if (!first) {
                    str_append(out, out_cap, ", ");
                }
                str_append(out, out_cap, one);
                first = 0;
            }
        }
        p = obj_end + 1;
    }
}

static size_t parse_ifaces(const char *json, iface_row *rows, size_t cap) {
    const char *items = strstr(json, "\"items\"");
    if (items == NULL) {
        return 0;
    }
    const char *p = strchr(items, '[');
    if (p == NULL) {
        return 0;
    }
    p++;

    size_t n = 0;
    while (*p != '\0' && n < cap) {
        p = skip_ws(p);
        if (*p == ']') {
            break;
        }
        if (*p != '{') {
            p++;
            continue;
        }
        const char *obj_start = p;
        const char *obj_end = find_object_end(obj_start);
        if (obj_end == NULL) {
            break;
        }

        iface_row r;
        r.if_index = 0;
        r.name[0] = '\0';
        r.ipv4[0] = '\0';
        r.ipv6[0] = '\0';

        const char *idx_p = find_key(obj_start, "if_index");
        if (idx_p != NULL) {
            (void)parse_u32_after_colon(idx_p, &r.if_index);
        }
        const char *name_p = find_key(obj_start, "name");
        if (name_p != NULL) {
            (void)parse_json_string(name_p, r.name, sizeof(r.name));
        }
        parse_ip_array(obj_start, "ipv4", r.ipv4, sizeof(r.ipv4));
        parse_ip_array(obj_start, "ipv6", r.ipv6, sizeof(r.ipv6));

        rows[n++] = r;
        p = obj_end + 1;
    }
    return n;
}

static int save_bytes(const char *path, const uint8_t *buf, size_t len) {
    FILE *fp = fopen(path, "wb");
    if (fp == NULL) {
        return 0;
    }
    (void)fwrite(buf, 1, len, fp);
    (void)fwrite("\n", 1, 1, fp);
    (void)fclose(fp);
    return 1;
}

static int fetch_list_json(tool_netif_list_json_fn f, tool_free_fn freer, uint8_t **out_buf, size_t *out_len) {
    *out_buf = NULL;
    *out_len = 0;
    int32_t rc = f(out_buf, out_len);
    return (int)rc;
}

static int run_apply(tool_netif_apply_json_fn f, tool_free_fn freer, const char *req_json, uint8_t **out_buf, size_t *out_len) {
    *out_buf = NULL;
    *out_len = 0;
    const uint8_t *p = (const uint8_t *)req_json;
    size_t n = strlen(req_json);
    int32_t rc = f(p, n, out_buf, out_len);
    return (int)rc;
}

int main(int argc, char **argv) {
    tool_netif_list_json_fn list_json = NULL;
    tool_netif_apply_json_fn apply_json = NULL;
    tool_free_fn free_fn = NULL;

    void *lib = load_library();

#if defined(_WIN32)
    (void)SetConsoleOutputCP(65001);
    (void)SetConsoleCP(65001);
#endif

#if defined(FORGEFFI_STATIC)
    list_json = tool_netif_list_json;
    apply_json = tool_netif_apply_json;
    free_fn = tool_free;
#else
    if (lib == NULL) {
        fprintf(stderr, "未能加载 ForgeFFI 动态库，请把 .dll/.so/.dylib 放到当前目录或 PATH/LD_LIBRARY_PATH 可找到的位置\n");
        return 2;
    }

    list_json = (tool_netif_list_json_fn)load_symbol(lib, "tool_netif_list_json");
    apply_json = (tool_netif_apply_json_fn)load_symbol(lib, "tool_netif_apply_json");
    free_fn = (tool_free_fn)load_symbol(lib, "tool_free");
#endif

    if (list_json == NULL || apply_json == NULL || free_fn == NULL) {
        fprintf(stderr, "missing symbols: tool_netif_list_json/tool_netif_apply_json/tool_free\n");
        close_library(lib);
        return 3;
    }

    (void)argc;
    (void)argv;

    uint8_t *last_json = NULL;
    size_t last_len = 0;

    for (;;) {
        fprintf(stderr, "\n=== ForgeFFI 网卡管理 demo ===\n");
        fprintf(stderr, "1) 刷新并显示全部网卡\n");
        fprintf(stderr, "2) 保存上次 JSON 到文件\n");
        fprintf(stderr, "3) 添加 IP 到网卡\n");
        fprintf(stderr, "4) 删除网卡上的 IP（支持删除 /0 用于清理误操作）\n");
        fprintf(stderr, "5) 替换网卡 IPv4（先删除该网卡所有 IPv4，再添加新 IPv4）\n");
        fprintf(stderr, "6) 设置 IPv4 DHCP 开/关（Linux 需要 NetworkManager）\n");
        fprintf(stderr, "0) 退出\n");
        fprintf(stderr, "> ");

        char line[1024];
        if (!read_line(line, sizeof(line))) {
            break;
        }

        int choice = atoi(line);
        if (choice == 0) {
            break;
        }

        if (choice == 1) {
            uint8_t *buf = NULL;
            size_t len = 0;
            int rc = fetch_list_json(list_json, free_fn, &buf, &len);
            fprintf(stderr, "rc=%d, out_len=%zu\n", rc, len);
            if (buf != NULL && len != 0) {
                if (last_json != NULL) {
                    free(last_json);
                    last_json = NULL;
                    last_len = 0;
                }
                last_json = (uint8_t *)malloc(len + 1);
                if (last_json != NULL) {
                    memcpy(last_json, buf, len);
                    last_json[len] = 0;
                    last_len = len;
                }

                iface_row rows[64];
                size_t n = parse_ifaces((const char *)buf, rows, (sizeof(rows) / sizeof(rows[0])));
                for (size_t i = 0; i < n; i++) {
                    const char *name = rows[i].name[0] ? rows[i].name : "(no name)";
                    const char *v4 = rows[i].ipv4[0] ? rows[i].ipv4 : "-";
                    const char *v6 = rows[i].ipv6[0] ? rows[i].ipv6 : "-";
                    fprintf(stdout, "[%zu] if_index=%u  名称=%s\n", i, (unsigned)rows[i].if_index, name);
                    fprintf(stdout, "     IPv4=%s\n", v4);
                    fprintf(stdout, "     IPv6=%s\n", v6);
                }

                free_fn(buf, len);
            }
            continue;
        }

        if (choice == 2) {
            if (last_json == NULL || last_len == 0) {
                fprintf(stderr, "没有缓存的 JSON，请先执行 1) 刷新\n");
                continue;
            }
            fprintf(stderr, "输出文件路径（默认: netif_list.json）: ");
            if (!read_line(line, sizeof(line))) {
                continue;
            }
            const char *path = (line[0] != '\0') ? line : "netif_list.json";
            if (save_bytes(path, last_json, last_len)) {
                fprintf(stderr, "已保存: %s\n", path);
            } else {
                fprintf(stderr, "保存失败: %s\n", path);
            }
            continue;
        }

        if (choice == 3 || choice == 4) {
            uint32_t if_index = 0;
            if (!ask_if_index(last_json, last_len, &if_index)) {
                fprintf(stderr, "if_index 无效\n");
                continue;
            }

            fprintf(stderr, "请输入 IP（例如 10.0.0.2 或 fe80::1）: ");
            if (!read_line(line, sizeof(line))) {
                continue;
            }
            char ip[128];
            ip[0] = '\0';
            strncpy(ip, line, sizeof(ip) - 1);
            ip[sizeof(ip) - 1] = '\0';
            if (ip[0] == '\0') {
                fprintf(stderr, "IP 无效\n");
                continue;
            }

            fprintf(stderr, "请输入 prefix_len（IPv4 0..=32, IPv6 0..=128；添加时建议 >=1）: ");
            if (!read_line(line, sizeof(line))) {
                continue;
            }
            uint32_t prefix = (uint32_t)strtoul(line, NULL, 10);
            if (is_ipv6_literal(ip)) {
                if (prefix > 128) {
                    fprintf(stderr, "prefix_len 超出范围（IPv6 0..=128）\n");
                    continue;
                }
            } else {
                if (prefix > 32) {
                    fprintf(stderr, "prefix_len 超出范围（IPv4 0..=32）\n");
                    continue;
                }
            }
            if (choice == 3 && prefix == 0) {
                fprintf(stderr, "添加 IP 不允许 prefix_len=0（这会导致非常怪异的行为）\n");
                continue;
            }

            char req[512];
            const char *op = (choice == 3) ? "add_ip" : "del_ip";
            if (snprintf(
                    req,
                    sizeof(req),
                    "{\"abi\":1,\"target\":{\"if_index\":%u},\"ops\":[{\"op\":\"%s\",\"ip\":\"%s\",\"prefix_len\":%u}]}",
                    (unsigned)if_index,
                    op,
                    ip,
                    (unsigned)prefix)
                <= 0) {
                fprintf(stderr, "构建请求失败\n");
                continue;
            }

            uint8_t *resp = NULL;
            size_t resp_len = 0;
            int rc = run_apply(apply_json, free_fn, req, &resp, &resp_len);
            fprintf(stderr, "rc=%d, out_len=%zu\n", rc, resp_len);
            if (resp != NULL && resp_len != 0) {
                (void)fwrite(resp, 1, resp_len, stdout);
                (void)fwrite("\n", 1, 1, stdout);
                (void)print_apply_response_pretty((const char *)resp);
                free_fn(resp, resp_len);
            }
            continue;
        }

        if (choice == 5) {
            uint32_t if_index = 0;
            if (!ask_if_index(last_json, last_len, &if_index)) {
                fprintf(stderr, "if_index 无效\n");
                continue;
            }

            fprintf(stderr, "请输入 IPv4 地址（例如 10.0.0.2）: ");
            if (!read_line(line, sizeof(line))) {
                continue;
            }
            char ip[64];
            ip[0] = '\0';
            strncpy(ip, line, sizeof(ip) - 1);
            ip[sizeof(ip) - 1] = '\0';
            if (ip[0] == '\0' || is_ipv6_literal(ip)) {
                fprintf(stderr, "IPv4 地址无效\n");
                continue;
            }

            fprintf(stderr, "请输入 prefix_len（IPv4 1..=32；例如 24 表示 255.255.255.0）: ");
            if (!read_line(line, sizeof(line))) {
                continue;
            }
            uint32_t prefix = (uint32_t)strtoul(line, NULL, 10);
            if (prefix == 0 || prefix > 32) {
                fprintf(stderr, "prefix_len 无效（IPv4 1..=32）\n");
                continue;
            }

            char gw[64];
            read_optional_gateway(gw, sizeof(gw));

            strbuf sb;
            sb.buf = NULL;
            sb.len = 0;
            sb.cap = 0;
            if (gw[0] != '\0') {
                if (!sb_appendf(
                        &sb,
                        "{\"abi\":1,\"target\":{\"if_index\":%u},\"ops\":[{\"op\":\"set_ipv4_static\",\"ip\":\"%s\",\"prefix_len\":%u,\"gateway\":\"%s\"}]}",
                        (unsigned)if_index,
                        ip,
                        (unsigned)prefix,
                        gw)) {
                    fprintf(stderr, "构建请求失败\n");
                    sb_free(&sb);
                    continue;
                }
            } else {
                if (!sb_appendf(
                        &sb,
                        "{\"abi\":1,\"target\":{\"if_index\":%u},\"ops\":[{\"op\":\"set_ipv4_static\",\"ip\":\"%s\",\"prefix_len\":%u}]}",
                        (unsigned)if_index,
                        ip,
                        (unsigned)prefix)) {
                    fprintf(stderr, "构建请求失败\n");
                    sb_free(&sb);
                    continue;
                }
            }

            uint8_t *resp = NULL;
            size_t resp_len = 0;
            int rc = run_apply(apply_json, free_fn, sb.buf, &resp, &resp_len);
            fprintf(stderr, "rc=%d, out_len=%zu\n", rc, resp_len);
            if (resp != NULL && resp_len != 0) {
                (void)fwrite(resp, 1, resp_len, stdout);
                (void)fwrite("\n", 1, 1, stdout);
                (void)print_apply_response_pretty((const char *)resp);
                free_fn(resp, resp_len);
            }
            sb_free(&sb);
            continue;
        }

        if (choice == 6) {
            uint32_t if_index = 0;
            if (!ask_if_index(last_json, last_len, &if_index)) {
                fprintf(stderr, "if_index 无效\n");
                continue;
            }
            fprintf(stderr, "是否启用 DHCP？(1=启用, 0=禁用): ");
            if (!read_line(line, sizeof(line))) {
                continue;
            }
            int enable = atoi(line) ? 1 : 0;

            char req[256];
            if (snprintf(
                    req,
                    sizeof(req),
                    "{\"abi\":1,\"target\":{\"if_index\":%u},\"ops\":[{\"op\":\"set_ipv4_dhcp\",\"enable\":%s}]}",
                    (unsigned)if_index,
                    enable ? "true" : "false")
                <= 0) {
                fprintf(stderr, "构建请求失败\n");
                continue;
            }

            uint8_t *resp = NULL;
            size_t resp_len = 0;
            int rc = run_apply(apply_json, free_fn, req, &resp, &resp_len);
            fprintf(stderr, "rc=%d, out_len=%zu\n", rc, resp_len);
            if (resp != NULL && resp_len != 0) {
                (void)fwrite(resp, 1, resp_len, stdout);
                (void)fwrite("\n", 1, 1, stdout);
                (void)print_apply_response_pretty((const char *)resp);
                free_fn(resp, resp_len);
            }
            continue;
        }

        fprintf(stderr, "未知选项\n");
    }

    if (last_json != NULL) {
        free(last_json);
        last_json = NULL;
        last_len = 0;
    }

    close_library(lib);
    return 0;
}
