#define JADREN_FILE_RUNTIME_HAS_STACK_SUPPORT 1
#define JADREN_FILE_RUNTIME_HAS_NETWORK_SUPPORT 0
#define JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT 0
#define JADREN_FILE_RUNTIME_HAS_UI_BINDINGS 0

typedef unsigned int UINT;
typedef unsigned long DWORD;
typedef long long LONGLONG;
typedef unsigned short wchar_t;
typedef void *HANDLE;
typedef const wchar_t *LPCWSTR;
unsigned __int64 app_data_revision(void);
unsigned __int64 app_data_snapshot_length(void);
unsigned __int64 app_data_snapshot_length_if_revision(unsigned __int64 expected_revision);
typedef struct JadrenFindDataW {
    DWORD file_attributes;
    DWORD creation_time_low;
    DWORD creation_time_high;
    DWORD last_access_time_low;
    DWORD last_access_time_high;
    DWORD last_write_time_low;
    DWORD last_write_time_high;
    DWORD file_size_high;
    DWORD file_size_low;
    DWORD reserved0;
    DWORD reserved1;
    wchar_t file_name[260];
    wchar_t alternate_file_name[14];
} JadrenFindDataW;

/* File/app-data storage is deliberately independent from the native desktop
 * host. A desktop build opts into this small callback only for restore
 * boundaries: successful load and rollback. Normal setters still require the
 * explicit ui_refresh_bindings() boundary, so there is no hidden reactive
 * graph or refresh on every model write. */
#if JADREN_FILE_RUNTIME_HAS_UI_BINDINGS
extern void ui_refresh_bindings(void);
static void app_data_refresh_bound_ui(void) {
    ui_refresh_bindings();
}
#else
static void app_data_refresh_bound_ui(void) {
}
#endif

#if JADREN_FILE_RUNTIME_HAS_NETWORK_SUPPORT
typedef unsigned long long JadrenSocket;
typedef struct JadrenSockaddrIn {
    unsigned short family;
    unsigned short port;
    unsigned char address[4];
    unsigned char zero[8];
} JadrenSockaddrIn;
typedef struct JadrenAddrInfo {
    int flags;
    int family;
    int socket_type;
    int protocol;
    unsigned long long address_length;
    char *canonical_name;
    void *address;
    struct JadrenAddrInfo *next;
} JadrenAddrInfo;
extern int WSAStartup(unsigned short version, unsigned char *data);
extern int WSACleanup(void);
extern int WSAGetLastError(void);
extern int getaddrinfo(const char *node, const char *service,
                       const JadrenAddrInfo *hints, JadrenAddrInfo **result);
extern void freeaddrinfo(JadrenAddrInfo *result);
extern JadrenSocket socket(int family, int type, int protocol);
extern int bind(JadrenSocket socket_handle, const void *address, int address_length);
extern int listen(JadrenSocket socket_handle, int backlog);
extern JadrenSocket accept(JadrenSocket socket_handle, void *address, int *address_length);
extern int connect(JadrenSocket socket_handle, const void *address, int address_length);
extern int ioctlsocket(JadrenSocket socket_handle, unsigned long command,
                       unsigned long *value);
extern int setsockopt(JadrenSocket socket_handle, int level, int option,
                      const char *value, int value_length);
extern int getsockopt(JadrenSocket socket_handle, int level, int option,
                      char *value, int *value_length);
extern int send(JadrenSocket socket_handle, const char *data, int length, int flags);
extern int recv(JadrenSocket socket_handle, char *data, int length, int flags);
extern int WSARecv(JadrenSocket socket_handle, void *buffers, unsigned long buffer_count,
                   unsigned long *received, unsigned long *flags,
                   void *overlapped, void *completion_routine);
extern int WSASend(JadrenSocket socket_handle, const void *buffers,
                   unsigned long buffer_count, unsigned long *sent,
                   unsigned long flags, void *overlapped, void *completion_routine);
extern int WSAConnect(JadrenSocket socket_handle, const void *address,
                      int address_length, void *call_data, void *callee_data,
                      void *security_qos, void *overlapped);
extern int WSAIoctl(JadrenSocket socket_handle, unsigned long control_code,
                    void *input_data, unsigned long input_length,
                    void *output_data, unsigned long output_length,
                    unsigned long *returned_length, void *overlapped,
                    void *completion_routine);
extern int closesocket(JadrenSocket socket_handle);
extern HANDLE CreateIoCompletionPort(HANDLE file_handle, HANDLE existing_port,
                                     unsigned long long completion_key,
                                     unsigned long concurrent_threads);
extern int GetQueuedCompletionStatus(HANDLE completion_port,
                                      unsigned long *bytes_transferred,
                                      unsigned long long *completion_key,
                                      void **overlapped, unsigned long timeout_ms);
extern int CancelIoEx(HANDLE file_handle, void *overlapped);
extern int CloseHandle(HANDLE handle);
extern int FlushFileBuffers(HANDLE handle);
extern unsigned long GetLastError(void);
extern unsigned long long GetTickCount64(void);
typedef struct JadrenFdSet {
    unsigned int count;
    JadrenSocket sockets[64];
} JadrenFdSet;
typedef struct JadrenTimeval {
    long seconds;
    long microseconds;
} JadrenTimeval;
extern int select(int nfds, JadrenFdSet *read_set, void *write_set,
                  void *except_set, JadrenTimeval *timeout);
typedef struct JadrenOverlapped {
    unsigned long long internal;
    unsigned long long internal_high;
    unsigned long offset;
    unsigned long offset_high;
    HANDLE event;
} JadrenOverlapped;
typedef struct JadrenWsabuf {
    unsigned long length;
    char *data;
} JadrenWsabuf;
typedef struct JadrenGuid {
    unsigned long data1;
    unsigned short data2;
    unsigned short data3;
    unsigned char data4[8];
} JadrenGuid;
typedef int (*JadrenAcceptExFn)(JadrenSocket listener, JadrenSocket accepted,
                                void *output_buffer, unsigned long receive_length,
                                unsigned long local_address_length,
                                unsigned long remote_address_length,
                                unsigned long *received,
                                JadrenOverlapped *overlapped);
typedef int (*JadrenConnectExFn)(JadrenSocket socket_handle,
                                 const void *address, int address_length,
                                 const void *send_data, unsigned long send_length,
                                 unsigned long *sent, JadrenOverlapped *overlapped);
#define JADREN_SIO_GET_EXTENSION_FUNCTION_POINTER 0xC8000006UL
#define JADREN_FIONBIO 0x8004667EUL
#define JADREN_WSA_IO_PENDING 997
#define JADREN_WSA_EWOULDBLOCK 10035
#define JADREN_WSA_EINPROGRESS 10036
#define JADREN_ERROR_OPERATION_ABORTED 995
#define JADREN_WAIT_TIMEOUT 258
#define JADREN_REACTOR_OPERATION_ACCEPT 1U
#define JADREN_REACTOR_OPERATION_CONNECT 2U
#define JADREN_REACTOR_OPERATION_RECEIVE 3U
#define JADREN_REACTOR_OPERATION_SEND 4U
static const JadrenGuid jadren_accept_ex_guid = {
    0xB5367DF1UL, 0xCBACU, 0x11CFU,
    {0x95U, 0xCAU, 0x00U, 0x80U, 0x5FU, 0x48U, 0xA1U, 0x92U}
};
static const JadrenGuid jadren_connect_ex_guid = {
    0x25A207B9UL, 0xDDF3U, 0x4660U,
    {0x8EU, 0xE9U, 0x76U, 0xE5U, 0x8CU, 0x74U, 0x06U, 0x3EU}
};
static int jadren_net_started = 0;
static unsigned int jadren_net_socket_count = 0;
static unsigned int jadren_net_reactor_count = 0;
#endif

#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
typedef struct JadrenSecHandle {
    unsigned long long lower;
    unsigned long long upper;
} JadrenSecHandle;
typedef struct JadrenTimeStamp {
    long long value;
} JadrenTimeStamp;
typedef struct JadrenSecBuffer {
    unsigned long length;
    unsigned long type;
    void *data;
} JadrenSecBuffer;
typedef struct JadrenSecBufferDesc {
    unsigned long version;
    unsigned long count;
    JadrenSecBuffer *buffers;
} JadrenSecBufferDesc;
typedef struct JadrenSchannelCred {
    unsigned long version;
    unsigned long credential_count;
    void *certificates;
    void *root_store;
    unsigned long mapper_count;
    void *mappers;
    unsigned long supported_algorithm_count;
    void *supported_algorithms;
    unsigned long enabled_protocols;
    unsigned long minimum_cipher_strength;
    unsigned long maximum_cipher_strength;
    unsigned long session_lifespan;
    unsigned long flags;
    unsigned long credential_format;
} JadrenSchannelCred;
typedef struct JadrenStreamSizes {
    unsigned long header;
    unsigned long trailer;
    unsigned long maximum_message;
    unsigned long buffers;
    unsigned long block_size;
} JadrenStreamSizes;
extern int AcquireCredentialsHandleW(const wchar_t *principal,
                                     const wchar_t *package,
                                     unsigned long credential_use,
                                     void *logon_id, void *auth_data,
                                     void *get_key_fn, void *get_key_arg,
                                     JadrenSecHandle *credential,
                                     JadrenTimeStamp *expiry);
extern int FreeCredentialsHandle(JadrenSecHandle *credential);
extern int InitializeSecurityContextW(
    JadrenSecHandle *credential, JadrenSecHandle *context,
    wchar_t *target_name, unsigned long context_requirements,
    unsigned long reserved1, unsigned long target_data_representation,
    JadrenSecBufferDesc *input, unsigned long reserved2,
    JadrenSecHandle *new_context, JadrenSecBufferDesc *output,
    unsigned long *context_attributes, JadrenTimeStamp *expiry);
extern int AcceptSecurityContext(
    JadrenSecHandle *credential, JadrenSecHandle *context,
    JadrenSecBufferDesc *input, unsigned long context_requirements,
    unsigned long target_data_representation, JadrenSecHandle *new_context,
    JadrenSecBufferDesc *output, unsigned long *context_attributes,
    JadrenTimeStamp *expiry);
extern int CompleteAuthToken(JadrenSecHandle *context, JadrenSecBufferDesc *token);
extern int DeleteSecurityContext(JadrenSecHandle *context);
extern int QueryContextAttributesW(JadrenSecHandle *context,
                                   unsigned long attribute, void *buffer);
extern int EncryptMessage(JadrenSecHandle *context, unsigned long quality_of_protection,
                          JadrenSecBufferDesc *message, unsigned long sequence_number);
extern int DecryptMessage(JadrenSecHandle *context, JadrenSecBufferDesc *message,
                          unsigned long sequence_number, unsigned long *quality_of_protection);
typedef struct JadrenCryptDataBlob {
    unsigned long length;
    unsigned char *data;
} JadrenCryptDataBlob;
typedef void *JadrenCertStore;
typedef struct JadrenCertContext {
    unsigned long encoding_type;
    unsigned char *encoded;
    unsigned long encoded_length;
    void *info;
    JadrenCertStore store;
} JadrenCertContext;
extern JadrenCertStore PFXImportCertStore(const JadrenCryptDataBlob *blob,
                                          const wchar_t *password,
                                          unsigned long flags);
extern JadrenCertContext *CertEnumCertificatesInStore(
    JadrenCertStore store, const JadrenCertContext *previous);
extern int CertCloseStore(JadrenCertStore store, unsigned long flags);
#endif

typedef struct JadrenLargeInteger {
    LONGLONG QuadPart;
} JadrenLargeInteger;

typedef struct JadrenFileTime {
    DWORD low;
    DWORD high;
} JadrenFileTime;

typedef unsigned long long SIZE_T;
extern HANDLE GetProcessHeap(void);
extern void *HeapAlloc(HANDLE heap, DWORD flags, SIZE_T bytes);
extern void *HeapReAlloc(HANDLE heap, DWORD flags, void *memory, SIZE_T bytes);
extern int HeapFree(HANDLE heap, DWORD flags, void *memory);

int _fltused = 0;

extern void ExitProcess(UINT result);

#if JADREN_FILE_RUNTIME_HAS_STACK_SUPPORT
__asm__(
    ".text\n"
    ".globl __chkstk\n"
    ".p2align 4, 0x90\n"
    "__chkstk:\n"
    "  pushq %rcx\n"
    "  pushq %rax\n"
    "  cmpq $4096, %rax\n"
    "  leaq 24(%rsp), %rcx\n"
    "  jb .Ljadren_file_chkstk_done\n"
    ".Ljadren_file_chkstk_loop:\n"
    "  subq $4096, %rcx\n"
    "  testq %rax, (%rcx)\n"
    "  subq $4096, %rax\n"
    "  cmpq $4096, %rax\n"
    "  jae .Ljadren_file_chkstk_loop\n"
    ".Ljadren_file_chkstk_done:\n"
    "  subq %rax, %rcx\n"
    "  testq %rax, (%rcx)\n"
    "  popq %rax\n"
    "  popq %rcx\n"
    "  retq\n");
#endif

void jadren_rt_bounds_panic_u64(unsigned __int64 index, unsigned __int64 length) {
    (void)index;
    (void)length;
    ExitProcess(1);
}

extern int MultiByteToWideChar(UINT code_page, DWORD flags, const char *source,
                               int source_length, wchar_t *target, int target_length);
extern int WideCharToMultiByte(UINT code_page, DWORD flags, const wchar_t *source,
                               int source_length, char *target, int target_length,
                               const char *default_character, int *used_default);
extern LPCWSTR GetCommandLineW(void);
extern HANDLE CreateFileW(LPCWSTR filename, DWORD desired_access, DWORD share_mode,
                          void *security_attributes, DWORD creation_disposition,
                          DWORD flags_and_attributes, HANDLE template_file);
extern int CloseHandle(HANDLE handle);
extern int FlushFileBuffers(HANDLE handle);
extern int ReadFile(HANDLE handle, void *buffer, DWORD length,
                    DWORD *read_count, void *overlapped);
extern int SetFilePointerEx(HANDLE handle, JadrenLargeInteger distance,
                            JadrenLargeInteger *new_position, DWORD move_method);
extern int WriteFile(HANDLE handle, const void *buffer, DWORD length,
                     DWORD *written_count, void *overlapped);
extern int GetFileSizeEx(HANDLE handle, JadrenLargeInteger *size);
extern DWORD GetFileAttributesW(LPCWSTR filename);
extern int CreateDirectoryW(LPCWSTR path, void *security_attributes);
extern int RemoveDirectoryW(LPCWSTR path);
extern HANDLE FindFirstFileW(LPCWSTR pattern, JadrenFindDataW *data);
extern int FindNextFileW(HANDLE search_handle, JadrenFindDataW *data);
extern int FindClose(HANDLE search_handle);
extern int CopyFileW(LPCWSTR existing_filename, LPCWSTR new_filename,
                     int fail_if_exists);
extern int DeleteFileW(LPCWSTR filename);
extern int MoveFileExW(LPCWSTR existing_filename, LPCWSTR new_filename, DWORD flags);
extern void GetSystemTimeAsFileTime(JadrenFileTime *file_time);
extern int QueryPerformanceCounter(JadrenLargeInteger *counter);
extern int QueryPerformanceFrequency(JadrenLargeInteger *frequency);
extern void Sleep(unsigned long milliseconds);

enum {
    CP_UTF8 = 65001,
    GENERIC_READ = 0x80000000U,
    GENERIC_WRITE = 0x40000000U,
    FILE_APPEND_DATA = 0x00000004U,
    FILE_ATTRIBUTE_DIRECTORY = 0x00000010U,
    FILE_SHARE_READ = 0x00000001U,
    FILE_SHARE_WRITE = 0x00000002U,
    FILE_SHARE_DELETE = 0x00000004U,
    CREATE_ALWAYS = 2,
    OPEN_EXISTING = 3,
    OPEN_ALWAYS = 4,
    FILE_BEGIN = 0,
    FILE_ATTRIBUTE_NORMAL = 0x00000080U,
    FILE_FLAG_BACKUP_SEMANTICS = 0x02000000U,
    MOVEFILE_REPLACE_EXISTING = 0x00000001U,
    MOVEFILE_WRITE_THROUGH = 0x00000008U,
    INVALID_FILE_ATTRIBUTES = 0xFFFFFFFFU,
    SOL_SOCKET = 0xFFFF,
    SO_SNDTIMEO = 0x1005,
    SO_RCVTIMEO = 0x1006,
    SO_ERROR = 0x1007,
    SO_UPDATE_ACCEPT_CONTEXT = 0x700B,
    SO_UPDATE_CONNECT_CONTEXT = 0x7010,
};

#define INVALID_HANDLE_VALUE ((HANDLE)(long long)-1)

static int path_to_wide(const char *path_data, unsigned __int64 path_length,
                        wchar_t *target, int capacity) {
    int source_length;
    int converted;
    if (target == 0 || capacity < 2 || path_data == 0 || path_length == 0 ||
        path_length > 0x7FFFFFFFULL) {
        return 0;
    }
    source_length = (int)path_length;
    converted = MultiByteToWideChar(CP_UTF8, 0, path_data, source_length,
                                    target, capacity - 1);
    if (converted <= 0 || converted >= capacity) {
        return 0;
    }
    target[converted] = 0;
    return 1;
}

/* The Windows command line is a single UTF-16 string rather than an argv
 * array.  Keep the parsing local to the file runtime so process arguments do
 * not require a second platform ABI.  The scanner follows the usual CRT
 * backslash/quote rules and removes syntactic quotes before UTF-8 conversion.
 */
static wchar_t jadren_process_arg_wide_buffer[32768];

static int jadren_process_arg_append(wchar_t *output, int capacity, int *length,
                                     wchar_t value) {
    if (*length >= 32767 ||
        (output != 0 && (capacity <= 0 || *length >= capacity - 1))) {
        return 0;
    }
    if (output != 0) {
        output[*length] = value;
    }
    *length += 1;
    return 1;
}

static int jadren_process_arg_wide(unsigned __int64 requested,
                                   wchar_t *output, int capacity) {
    LPCWSTR cursor = GetCommandLineW();
    unsigned __int64 current = 0;
    if (cursor == 0) {
        return -1;
    }
    for (;;) {
        int in_quotes = 0;
        int length = 0;
        int store;
        while (*cursor == (wchar_t)' ' || *cursor == (wchar_t)'\t') {
            cursor += 1;
        }
        if (*cursor == 0) {
            return -1;
        }
        store = current == requested;
        for (;;) {
            unsigned int slashes = 0;
            unsigned int index;
            while (cursor[slashes] == (wchar_t)'\\') {
                slashes += 1U;
            }
            if (cursor[slashes] == (wchar_t)'"') {
                for (index = 0; index < slashes / 2U; index += 1U) {
                    if (store && !jadren_process_arg_append(output, capacity,
                                                            &length, (wchar_t)'\\')) {
                        return -2;
                    }
                }
                cursor += slashes;
                if ((slashes & 1U) != 0U) {
                    cursor += 1;
                    if (store && !jadren_process_arg_append(output, capacity,
                                                            &length, (wchar_t)'"')) {
                        return -2;
                    }
                } else {
                    cursor += 1;
                    if (in_quotes && *cursor == (wchar_t)'"') {
                        if (store && !jadren_process_arg_append(
                                output, capacity, &length, (wchar_t)'"')) {
                            return -2;
                        }
                        cursor += 1;
                    } else {
                        in_quotes = !in_quotes;
                    }
                }
            } else {
                for (index = 0; index < slashes; index += 1U) {
                    if (store && !jadren_process_arg_append(output, capacity,
                                                            &length, (wchar_t)'\\')) {
                        return -2;
                    }
                }
                cursor += slashes;
                if (*cursor == 0 ||
                    (!in_quotes && (*cursor == (wchar_t)' ' ||
                                    *cursor == (wchar_t)'\t'))) {
                    break;
                }
                if (store && !jadren_process_arg_append(output, capacity,
                                                        &length, *cursor)) {
                    return -2;
                }
                cursor += 1;
            }
        }
        if (store) {
            if (output != 0) {
                output[length] = 0;
            }
            return length;
        }
        current += 1;
    }
}

unsigned __int64 process_arg_count(void) {
    unsigned __int64 count = 0;
    while (jadren_process_arg_wide(count, 0, 0) >= 0) {
        count += 1;
    }
    return count;
}

unsigned __int64 process_arg_read(unsigned __int64 index,
                                  unsigned char *output_data,
                                  unsigned __int64 output_length) {
    int wide_length = jadren_process_arg_wide(
        index, jadren_process_arg_wide_buffer,
        (int)(sizeof(jadren_process_arg_wide_buffer) /
              sizeof(jadren_process_arg_wide_buffer[0])));
    int required;
    int converted;
    if (wide_length < 0) {
        return 0;
    }
    required = WideCharToMultiByte(CP_UTF8, 0, jadren_process_arg_wide_buffer,
                                   wide_length, 0, 0, 0, 0);
    if (required <= 0 || (unsigned __int64)required > output_length ||
        (required > 0 && output_data == 0)) {
        return 0;
    }
    converted = WideCharToMultiByte(CP_UTF8, 0, jadren_process_arg_wide_buffer,
                                    wide_length, (char *)output_data, required,
                                    0, 0);
    return converted == required ? (unsigned __int64)converted : 0;
}

#if JADREN_FILE_RUNTIME_HAS_NETWORK_SUPPORT
static unsigned short jadren_net_htons(unsigned short value) {
    return (unsigned short)((value << 8) | (value >> 8));
}

static int jadren_net_parse_ipv4(const char *host_data,
                                 unsigned __int64 host_length,
                                 unsigned char address[4]) {
    unsigned int part = 0;
    unsigned int digits = 0;
    unsigned int component = 0;
    unsigned __int64 index;
    if (host_data == 0 || host_length == 0 || address == 0) {
        return 0;
    }
    if (host_length == 9 && host_data[0] == 'l' && host_data[1] == 'o' &&
        host_data[2] == 'c' && host_data[3] == 'a' && host_data[4] == 'l' &&
        host_data[5] == 'h' && host_data[6] == 'o' && host_data[7] == 's' &&
        host_data[8] == 't') {
        address[0] = 127;
        address[1] = 0;
        address[2] = 0;
        address[3] = 1;
        return 1;
    }
    for (index = 0; index < host_length; index += 1) {
        unsigned char value = (unsigned char)host_data[index];
        if (value >= (unsigned char)'0' && value <= (unsigned char)'9') {
            if (digits >= 3) {
                return 0;
            }
            part = part * 10U + (unsigned int)(value - (unsigned char)'0');
            if (part > 255U) {
                return 0;
            }
            digits += 1;
        } else if (value == (unsigned char)'.' && digits > 0 && component < 3) {
            address[component] = (unsigned char)part;
            component += 1;
            part = 0;
            digits = 0;
        } else {
            return -13;
        }
    }
    if (component != 3 || digits == 0) {
        return -13;
    }
    address[3] = (unsigned char)part;
    return 1;
}

static int jadren_net_startup(void) {
    unsigned char startup_data[400];
    if (!jadren_net_started && WSAStartup(0x0202U, startup_data) == 0) {
        jadren_net_started = 1;
    }
    return jadren_net_started;
}

static void jadren_net_maybe_cleanup(void) {
    if (jadren_net_started && jadren_net_socket_count == 0 &&
        jadren_net_reactor_count == 0) {
        (void)WSACleanup();
        jadren_net_started = 0;
    }
}

static void jadren_net_loopback_address(JadrenSockaddrIn *address,
                                        unsigned short port) {
    unsigned int index;
    address->family = 2;
    address->port = jadren_net_htons(port);
    address->address[0] = 127;
    address->address[1] = 0;
    address->address[2] = 0;
    address->address[3] = 1;
    for (index = 0; index < 8; index += 1) {
        address->zero[index] = 0;
    }
}

unsigned __int64 net_tcp_connect(const char *host_data,
                                 unsigned __int64 host_length,
                                 unsigned short port) {
    JadrenSockaddrIn address;
    unsigned char parsed_address[4];
    JadrenSocket socket_handle;
    if (!jadren_net_startup() ||
        !jadren_net_parse_ipv4(host_data, host_length, parsed_address)) {
        return -14;
    }
    address.family = 2;
    address.port = jadren_net_htons(port);
    address.address[0] = parsed_address[0];
    address.address[1] = parsed_address[1];
    address.address[2] = parsed_address[2];
    address.address[3] = parsed_address[3];
    for (unsigned int index = 0; index < 8; index += 1) {
        address.zero[index] = 0;
    }
    socket_handle = socket(2, 1, 6);
    if (socket_handle == (JadrenSocket)-1 ||
        connect(socket_handle, &address, 16) != 0) {
        if (socket_handle != (JadrenSocket)-1) {
            closesocket(socket_handle);
        }
        return 0;
    }
    jadren_net_socket_count += 1;
    return (unsigned __int64)socket_handle + 1ULL;
}

unsigned __int64 net_tcp_connect_dns(const char *host_data,
                                     unsigned __int64 host_length,
                                     unsigned short port) {
    char host[256];
    JadrenAddrInfo hints;
    JadrenAddrInfo *result;
    JadrenSockaddrIn *resolved;
    JadrenSockaddrIn address;
    JadrenSocket socket_handle;
    unsigned __int64 index;
    if (!jadren_net_startup() || host_data == 0 || host_length == 0 ||
        host_length >= sizeof(host)) {
        return 0;
    }
    for (index = 0; index < host_length; index += 1) {
        if (host_data[index] == 0) {
            return 0;
        }
        host[index] = host_data[index];
    }
    host[host_length] = 0;
    hints.flags = 0;
    hints.family = 2;
    hints.socket_type = 1;
    hints.protocol = 6;
    hints.address_length = 0;
    hints.canonical_name = 0;
    hints.address = 0;
    hints.next = 0;
    result = 0;
    if (getaddrinfo(host, 0, &hints, &result) != 0 || result == 0 ||
        result->address == 0) {
        if (result != 0) {
            freeaddrinfo(result);
        }
        return 0;
    }
    resolved = (JadrenSockaddrIn *)result->address;
    address.family = 2;
    address.port = jadren_net_htons(port);
    address.address[0] = resolved->address[0];
    address.address[1] = resolved->address[1];
    address.address[2] = resolved->address[2];
    address.address[3] = resolved->address[3];
    for (index = 0; index < 8; index += 1) {
        address.zero[index] = 0;
    }
    freeaddrinfo(result);
    socket_handle = socket(2, 1, 6);
    if (socket_handle == (JadrenSocket)-1 ||
        connect(socket_handle, &address, 16) != 0) {
        if (socket_handle != (JadrenSocket)-1) {
            closesocket(socket_handle);
        }
        return 0;
    }
    jadren_net_socket_count += 1;
    return (unsigned __int64)socket_handle + 1ULL;
}

unsigned __int64 net_tcp_listen(unsigned short port) {
    JadrenSockaddrIn address;
    JadrenSocket socket_handle;
    if (!jadren_net_startup()) {
        return 0;
    }
    socket_handle = socket(2, 1, 6);
    if (socket_handle == (JadrenSocket)-1) {
        return 0;
    }
    jadren_net_loopback_address(&address, port);
    if (bind(socket_handle, &address, 16) != 0 || listen(socket_handle, 8) != 0) {
        closesocket(socket_handle);
        return 0;
    }
    jadren_net_socket_count += 1;
    return (unsigned __int64)socket_handle + 1ULL;
}

unsigned __int64 net_tcp_listen_on(const char *address_data,
                                   unsigned __int64 address_length,
                                   unsigned short port) {
    JadrenSockaddrIn address;
    unsigned char parsed_address[4];
    JadrenSocket socket_handle;
    unsigned int index;
    if (!jadren_net_startup() ||
        !jadren_net_parse_ipv4(address_data, address_length, parsed_address)) {
        return 0;
    }
    socket_handle = socket(2, 1, 6);
    if (socket_handle == (JadrenSocket)-1) {
        return 0;
    }
    address.family = 2;
    address.port = jadren_net_htons(port);
    address.address[0] = parsed_address[0];
    address.address[1] = parsed_address[1];
    address.address[2] = parsed_address[2];
    address.address[3] = parsed_address[3];
    for (index = 0; index < 8; index += 1) {
        address.zero[index] = 0;
    }
    if (bind(socket_handle, &address, 16) != 0 || listen(socket_handle, 8) != 0) {
        closesocket(socket_handle);
        return 0;
    }
    jadren_net_socket_count += 1;
    return (unsigned __int64)socket_handle + 1ULL;
}

unsigned __int64 net_tcp_accept(unsigned __int64 listener) {
    JadrenSocket listener_handle;
    JadrenSocket client_handle;
    if (listener == 0 || !jadren_net_startup()) {
        return 0;
    }
    listener_handle = (JadrenSocket)(listener - 1ULL);
    client_handle = accept(listener_handle, 0, 0);
    if (client_handle == (JadrenSocket)-1) {
        return 0;
    }
    jadren_net_socket_count += 1;
    return (unsigned __int64)client_handle + 1ULL;
}

unsigned __int64 net_tcp_send(unsigned __int64 socket_token,
                              const unsigned char *input_data,
                              unsigned __int64 input_length) {
    unsigned __int64 requested;
    int sent;
    if (socket_token == 0 || (input_data == 0 && input_length > 0)) {
        return 0;
    }
    requested = input_length > 0x7FFFFFFFULL ? 0x7FFFFFFFULL : input_length;
    sent = send((JadrenSocket)(socket_token - 1ULL), (const char *)input_data,
                (int)requested, 0);
    return sent > 0 ? (unsigned __int64)sent : 0;
}

unsigned __int64 net_tcp_send_prefix(unsigned __int64 socket_token,
                                     const unsigned char *input_data,
                                     unsigned __int64 input_length,
                                     unsigned __int64 send_length) {
    unsigned __int64 requested;
    int sent;
    if (socket_token == 0 || (input_data == 0 && send_length > 0) ||
        send_length > input_length || send_length == 0) {
        return 0;
    }
    requested = send_length > 0x7FFFFFFFULL ? 0x7FFFFFFFULL : send_length;
    sent = send((JadrenSocket)(socket_token - 1ULL), (const char *)input_data,
                (int)requested, 0);
    return sent > 0 ? (unsigned __int64)sent : 0;
}

unsigned __int64 net_tcp_send_all(unsigned __int64 socket_token,
                                  const unsigned char *input_data,
                                  unsigned __int64 input_length) {
    unsigned __int64 total = 0;
    while (total < input_length) {
        unsigned __int64 sent = net_tcp_send_prefix(socket_token,
                                                     input_data + total,
                                                     input_length - total,
                                                     input_length - total);
        if (sent == 0) return total;
        total += sent;
    }
    return total;
}

unsigned __int64 net_tcp_send_all_prefix(unsigned __int64 socket_token,
                                         const unsigned char *input_data,
                                         unsigned __int64 input_length,
                                         unsigned __int64 send_length) {
    if (send_length > input_length) return 0;
    return net_tcp_send_all(socket_token, input_data, send_length);
}

unsigned __int64 net_tcp_receive(unsigned __int64 socket_token,
                                  unsigned char *output_data,
                                  unsigned __int64 output_length) {
    unsigned __int64 requested;
    int received;
    if (socket_token == 0 || output_data == 0 || output_length == 0) {
        return 0;
    }
    requested = output_length > 0x7FFFFFFFULL ? 0x7FFFFFFFULL : output_length;
    received = recv((JadrenSocket)(socket_token - 1ULL), (char *)output_data,
                    (int)requested, 0);
    return received > 0 ? (unsigned __int64)received : 0;
}

int net_socket_set_timeout(unsigned __int64 socket_token,
                           unsigned int milliseconds) {
    int timeout;
    if (socket_token == 0) {
        return 0;
    }
    timeout = milliseconds > 0x7FFFFFFFU ? 0x7FFFFFFF : (int)milliseconds;
    return setsockopt((JadrenSocket)(socket_token - 1ULL), SOL_SOCKET,
                      SO_RCVTIMEO, (const char *)&timeout, sizeof(timeout)) == 0 &&
           setsockopt((JadrenSocket)(socket_token - 1ULL), SOL_SOCKET,
                      SO_SNDTIMEO, (const char *)&timeout, sizeof(timeout)) == 0;
}

int net_socket_close(unsigned __int64 socket_token) {
    int result;
    if (socket_token == 0) {
        return 0;
    }
    result = closesocket((JadrenSocket)(socket_token - 1ULL)) == 0;
    if (result && jadren_net_socket_count > 0) {
        jadren_net_socket_count -= 1;
        jadren_net_maybe_cleanup();
    }
    return result;
}

#define JADREN_REACTOR_CAPACITY 4
#define JADREN_REACTOR_WATCH_CAPACITY 64
#define JADREN_REACTOR_EVENT_CAPACITY 64
#define JADREN_REACTOR_READABLE 1U
#define JADREN_REACTOR_WRITABLE 2U
#define JADREN_REACTOR_ERROR 4U
#define JADREN_REACTOR_PEER_CLOSED 8U
#define JADREN_REACTOR_TIMEOUT 16U
#define JADREN_REACTOR_CANCELLED 32U
#define JADREN_REACTOR_OPERATION_TAG (1ULL << 63)
#define JADREN_REACTOR_OPERATION_ACCEPT 1U
#define JADREN_REACTOR_OPERATION_CONNECT 2U
#define JADREN_REACTOR_OPERATION_RECEIVE 3U
#define JADREN_REACTOR_OPERATION_SEND 4U

typedef struct JadrenReactorWatch {
    int active;
    unsigned __int64 socket_token;
    unsigned int interest;
    unsigned __int64 user_data;
} JadrenReactorWatch;

typedef struct JadrenReactorEvent {
    unsigned __int64 socket_token;
    unsigned __int64 operation_token;
    unsigned int flags;
    unsigned __int64 user_data;
    unsigned __int64 transferred;
} JadrenReactorEvent;

typedef struct JadrenReactorOperation {
    int active;
    int completed;
    int cancel_requested;
    int timed_out;
    int retired;
    unsigned int kind;
    unsigned __int64 socket_token;
    unsigned __int64 result_socket_token;
    unsigned __int64 user_data;
    int completion_error;
    unsigned __int64 transferred;
    unsigned char *buffer_data;
    unsigned __int64 buffer_length;
    unsigned long long deadline_tick;
    JadrenSocket operation_socket;
    JadrenOverlapped overlapped;
    unsigned char address_buffer[128];
    unsigned char io_buffer[1];
} JadrenReactorOperation;

typedef struct JadrenReactor {
    int active;
    HANDLE completion_port;
    unsigned int max_watches;
    unsigned int max_events;
    unsigned int event_count;
    int last_error;
    JadrenReactorOperation operations[JADREN_REACTOR_EVENT_CAPACITY];
    JadrenReactorWatch watches[JADREN_REACTOR_WATCH_CAPACITY];
    JadrenReactorEvent events[JADREN_REACTOR_EVENT_CAPACITY];
    unsigned int associated_socket_count;
    JadrenSocket associated_sockets[JADREN_REACTOR_WATCH_CAPACITY];
} JadrenReactor;

static JadrenReactor jadren_reactors[JADREN_REACTOR_CAPACITY];

static JadrenReactor *jadren_reactor_find(unsigned __int64 token) {
    if (token == 0 || token > JADREN_REACTOR_CAPACITY) return 0;
    if (!jadren_reactors[token - 1ULL].active) return 0;
    return &jadren_reactors[token - 1ULL];
}

static int jadren_reactor_find_watch(JadrenReactor *reactor,
                                     unsigned __int64 socket_token) {
    unsigned int index;
    for (index = 0; index < reactor->max_watches; index += 1) {
        if (reactor->watches[index].active &&
            reactor->watches[index].socket_token == socket_token) {
            return (int)index;
        }
    }
    return -1;
}

static int jadren_reactor_free_watch(JadrenReactor *reactor) {
    unsigned int index;
    for (index = 0; index < reactor->max_watches; index += 1) {
        if (!reactor->watches[index].active) return (int)index;
    }
    return -1;
}

static int jadren_reactor_free_operation(JadrenReactor *reactor) {
    unsigned int index;
    for (index = 0; index < reactor->max_events; index += 1) {
        if (!reactor->operations[index].active &&
            !reactor->operations[index].retired) return (int)index;
    }
    return -1;
}

static JadrenReactorOperation *jadren_reactor_find_operation(
    JadrenReactor *reactor, unsigned __int64 token) {
    if (token == 0 || token > reactor->max_events) return 0;
    if (!reactor->operations[token - 1ULL].active) return 0;
    return &reactor->operations[token - 1ULL];
}

static void jadren_reactor_prepare_operation(JadrenReactorOperation *operation) {
    operation->active = 1;
    operation->completed = 0;
    operation->cancel_requested = 0;
    operation->timed_out = 0;
    operation->retired = 0;
    operation->kind = 0;
    operation->socket_token = 0;
    operation->result_socket_token = 0;
    operation->user_data = 0;
    operation->completion_error = 0;
    operation->transferred = 0;
    operation->buffer_data = 0;
    operation->buffer_length = 0;
    operation->deadline_tick = 0;
    operation->operation_socket = (JadrenSocket)-1;
    operation->overlapped.internal = 0;
    operation->overlapped.internal_high = 0;
    operation->overlapped.offset = 0;
    operation->overlapped.offset_high = 0;
    operation->overlapped.event = 0;
}

static int jadren_reactor_associate_socket(JadrenReactor *reactor,
                                           JadrenSocket socket_handle) {
    HANDLE associated;
    unsigned int index;
    for (index = 0; index < reactor->associated_socket_count; index += 1) {
        if (reactor->associated_sockets[index] == socket_handle) return 1;
    }
    associated = CreateIoCompletionPort(
        (HANDLE)(long long)socket_handle, reactor->completion_port, 0, 0);
    if (associated == 0) {
        reactor->last_error = (int)GetLastError();
        return 0;
    }
    if (reactor->associated_socket_count >= JADREN_REACTOR_WATCH_CAPACITY) {
        reactor->last_error = 10055;
        return 0;
    }
    reactor->associated_sockets[reactor->associated_socket_count++] = socket_handle;
    return 1;
}

static int jadren_reactor_get_extension(JadrenSocket socket_handle,
                                        const JadrenGuid *guid,
                                        void **extension) {
    unsigned long bytes = 0;
    if (WSAIoctl(socket_handle, JADREN_SIO_GET_EXTENSION_FUNCTION_POINTER,
                 (void *)guid, sizeof(*guid), extension, sizeof(*extension),
                 &bytes, 0, 0) != 0) {
        return 0;
    }
    return *extension != 0;
}

unsigned __int64 net_reactor_submit_accept(unsigned __int64 reactor_token,
                                           unsigned __int64 listener_token,
                                           unsigned __int64 user_data) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    JadrenReactorOperation *operation;
    JadrenSocket listener;
    JadrenSocket accepted;
    JadrenAcceptExFn accept_ex = 0;
    unsigned long bytes = 0;
    int status;
    int index;
    if (reactor == 0 || listener_token == 0) return 0;
    index = jadren_reactor_free_operation(reactor);
    if (index < 0) {
        reactor->last_error = 10055;
        return 0;
    }
    listener = (JadrenSocket)(listener_token - 1ULL);
    if (!jadren_reactor_associate_socket(reactor, listener) ||
        !jadren_reactor_get_extension(listener, &jadren_accept_ex_guid,
                                      (void **)&accept_ex)) {
        reactor->last_error = WSAGetLastError();
        return 0;
    }
    accepted = socket(2, 1, 6);
    if (accepted == (JadrenSocket)-1) {
        reactor->last_error = WSAGetLastError();
        return 0;
    }
    operation = &reactor->operations[index];
    jadren_reactor_prepare_operation(operation);
    operation->kind = JADREN_REACTOR_OPERATION_ACCEPT;
    operation->socket_token = listener_token;
    operation->user_data = user_data;
    operation->operation_socket = accepted;
    status = accept_ex(listener, accepted, operation->address_buffer, 0,
                       32, 32, &bytes, &operation->overlapped);
    if (!status) {
        int error = WSAGetLastError();
        if (error != JADREN_WSA_IO_PENDING) {
            operation->active = 0;
            closesocket(accepted);
            reactor->last_error = error;
            return 0;
        }
    }
    jadren_net_socket_count += 1;
    reactor->last_error = 0;
    return (unsigned __int64)index + 1ULL;
}

unsigned __int64 net_reactor_submit_connect(
    unsigned __int64 reactor_token, const char *host_data,
    unsigned __int64 host_length, unsigned short port,
    unsigned __int64 user_data) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    JadrenReactorOperation *operation;
    JadrenSockaddrIn address;
    JadrenSockaddrIn local_address;
    unsigned char parsed_address[4];
    JadrenSocket socket_handle;
    JadrenConnectExFn connect_ex = 0;
    unsigned long bytes = 0;
    int index;
    int status;
    if (reactor == 0 || host_data == 0 || host_length == 0 ||
        !jadren_net_parse_ipv4(host_data, host_length, parsed_address)) {
        return 0;
    }
    index = jadren_reactor_free_operation(reactor);
    if (index < 0) {
        reactor->last_error = 10055;
        return 0;
    }
    address.family = 2;
    address.port = jadren_net_htons(port);
    address.address[0] = parsed_address[0];
    address.address[1] = parsed_address[1];
    address.address[2] = parsed_address[2];
    address.address[3] = parsed_address[3];
    for (unsigned int cursor = 0; cursor < 8; cursor += 1) {
        address.zero[cursor] = 0;
    }
    local_address.family = 2;
    local_address.port = 0;
    local_address.address[0] = 0;
    local_address.address[1] = 0;
    local_address.address[2] = 0;
    local_address.address[3] = 0;
    for (unsigned int cursor = 0; cursor < 8; cursor += 1) {
        local_address.zero[cursor] = 0;
    }
    socket_handle = socket(2, 1, 6);
    if (socket_handle == (JadrenSocket)-1 ||
        bind(socket_handle, &local_address, 16) != 0 ||
        !jadren_reactor_associate_socket(reactor, socket_handle) ||
        !jadren_reactor_get_extension(socket_handle, &jadren_connect_ex_guid,
                                      (void **)&connect_ex)) {
        if (socket_handle != (JadrenSocket)-1) closesocket(socket_handle);
        reactor->last_error = WSAGetLastError();
        return 0;
    }
    operation = &reactor->operations[index];
    jadren_reactor_prepare_operation(operation);
    operation->kind = JADREN_REACTOR_OPERATION_CONNECT;
    operation->socket_token = (unsigned __int64)socket_handle + 1ULL;
    operation->result_socket_token = operation->socket_token;
    operation->user_data = user_data;
    operation->operation_socket = socket_handle;
    status = connect_ex(socket_handle, &address, 16, 0, 0, &bytes,
                        &operation->overlapped);
    if (!status) {
        int error = WSAGetLastError();
        if (error != JADREN_WSA_IO_PENDING) {
            operation->active = 0;
            closesocket(socket_handle);
            reactor->last_error = error;
            return 0;
        }
    }
    jadren_net_socket_count += 1;
    reactor->last_error = 0;
    return (unsigned __int64)index + 1ULL;
}

static unsigned __int64 jadren_reactor_submit_io(
    JadrenReactor *reactor, unsigned __int64 socket_token,
    unsigned char *buffer_data, unsigned __int64 buffer_length,
    unsigned __int64 user_data, unsigned int kind) {
    JadrenReactorOperation *operation;
    JadrenSocket socket_handle;
    JadrenWsabuf buffer;
    unsigned long flags = 0;
    int status;
    int index;
    if (socket_token == 0 ||
        (buffer_length > 0 && buffer_data == 0) ||
        buffer_length > 0xFFFFFFFFULL) return 0;
    index = jadren_reactor_free_operation(reactor);
    if (index < 0) {
        reactor->last_error = 10055;
        return 0;
    }
    socket_handle = (JadrenSocket)(socket_token - 1ULL);
    operation = &reactor->operations[index];
    jadren_reactor_prepare_operation(operation);
    operation->kind = kind;
    operation->socket_token = socket_token;
    operation->result_socket_token = socket_token;
    operation->user_data = user_data;
    operation->operation_socket = socket_handle;
    operation->buffer_data = buffer_data;
    operation->buffer_length = buffer_length;
    if (!jadren_reactor_associate_socket(reactor, socket_handle)) {
        operation->active = 0;
        return 0;
    }
    buffer.length = (unsigned long)buffer_length;
    buffer.data = (char *)buffer_data;
    if (kind == JADREN_REACTOR_OPERATION_RECEIVE) {
        status = WSARecv(socket_handle, &buffer, 1, 0, &flags,
                         &operation->overlapped, 0);
    } else {
        status = WSASend(socket_handle, &buffer, 1, 0, 0,
                         &operation->overlapped, 0);
    }
    if (status != 0) {
        int error = WSAGetLastError();
        if (error != JADREN_WSA_IO_PENDING) {
            operation->active = 0;
            reactor->last_error = error;
            return 0;
        }
    }
    reactor->last_error = 0;
    return (unsigned __int64)index + 1ULL;
}

unsigned __int64 net_reactor_submit_receive(unsigned __int64 reactor_token,
                                            unsigned __int64 socket_token,
                                            unsigned __int64 user_data) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    if (reactor == 0) return 0;
    return jadren_reactor_submit_io(reactor, socket_token, 0, 0, user_data,
                                    JADREN_REACTOR_OPERATION_RECEIVE);
}

unsigned __int64 net_reactor_submit_send(unsigned __int64 reactor_token,
                                         unsigned __int64 socket_token,
                                         unsigned __int64 user_data) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    if (reactor == 0) return 0;
    return jadren_reactor_submit_io(reactor, socket_token, 0, 0, user_data,
                                    JADREN_REACTOR_OPERATION_SEND);
}

unsigned __int64 net_reactor_submit_receive_buffer(
    unsigned __int64 reactor_token, unsigned __int64 socket_token,
    unsigned char *output_data, unsigned __int64 output_length,
    unsigned __int64 user_data) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    if (reactor == 0) return 0;
    return jadren_reactor_submit_io(
        reactor, socket_token, output_data, output_length, user_data,
        JADREN_REACTOR_OPERATION_RECEIVE);
}

unsigned __int64 net_reactor_submit_send_buffer(
    unsigned __int64 reactor_token, unsigned __int64 socket_token,
    const unsigned char *input_data, unsigned __int64 input_length,
    unsigned __int64 user_data) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    if (reactor == 0) return 0;
    return jadren_reactor_submit_io(
        reactor, socket_token, (unsigned char *)input_data, input_length,
        user_data, JADREN_REACTOR_OPERATION_SEND);
}

unsigned __int64 net_reactor_submit_send_buffer_prefix(
    unsigned __int64 reactor_token, unsigned __int64 socket_token,
    const unsigned char *input_data, unsigned __int64 input_length,
    unsigned __int64 send_length, unsigned __int64 user_data) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    if (reactor == 0 || send_length == 0 || send_length > input_length) return 0;
    return jadren_reactor_submit_io(
        reactor, socket_token, (unsigned char *)input_data, send_length,
        user_data, JADREN_REACTOR_OPERATION_SEND);
}

int net_reactor_cancel(unsigned __int64 reactor_token,
                       unsigned __int64 operation_token) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    JadrenReactorOperation *operation;
    JadrenSocket cancel_socket;
    if (reactor == 0) return 0;
    operation = jadren_reactor_find_operation(reactor, operation_token);
    if (operation == 0 || operation->cancel_requested) return 0;
    if (operation->completed == 1) {
        if ((operation->kind == JADREN_REACTOR_OPERATION_ACCEPT ||
             operation->kind == JADREN_REACTOR_OPERATION_CONNECT) &&
            operation->operation_socket != (JadrenSocket)-1) {
            closesocket(operation->operation_socket);
            if (jadren_net_socket_count > 0) {
                jadren_net_socket_count -= 1;
            }
        }
        operation->active = 0;
        reactor->event_count = 0;
        reactor->last_error = 0;
        return 1;
    }
    if (operation->completed == 2) {
        if (operation->kind == JADREN_REACTOR_OPERATION_CONNECT &&
            operation->operation_socket != (JadrenSocket)-1) {
            closesocket(operation->operation_socket);
            if (jadren_net_socket_count > 0) {
                jadren_net_socket_count -= 1;
            }
        }
        operation->active = 0;
        reactor->event_count = 0;
        reactor->last_error = 0;
        return 1;
    }
    cancel_socket = operation->kind == JADREN_REACTOR_OPERATION_ACCEPT
        ? (JadrenSocket)(operation->socket_token - 1ULL)
        : operation->operation_socket;
    if (CancelIoEx((HANDLE)(long long)cancel_socket, &operation->overlapped) == 0) {
        int error = (int)GetLastError();
        if (error != JADREN_ERROR_OPERATION_ABORTED && error != 1168) {
            reactor->last_error = error;
            return 0;
        }
    }
    operation->cancel_requested = 1;
    // WSARecv/WSASend cancellation is not guaranteed to enqueue an IOCP
    // completion before the caller's next poll. Retire caller-buffered I/O
    // synchronously so cancel has a deterministic event contract; the
    // `retired` bit discards any late kernel completion safely.
    if (operation->kind == JADREN_REACTOR_OPERATION_RECEIVE ||
        operation->kind == JADREN_REACTOR_OPERATION_SEND) {
        operation->completed = 1;
        operation->completion_error = JADREN_ERROR_OPERATION_ABORTED;
        operation->retired = 1;
    }
    reactor->last_error = 0;
    return 1;
}

unsigned __int64 net_reactor_open(unsigned int max_watches,
                                  unsigned int max_events) {
    unsigned int index;
    if (!jadren_net_startup() || max_watches == 0 || max_events == 0 ||
        max_watches > JADREN_REACTOR_WATCH_CAPACITY ||
        max_events > JADREN_REACTOR_EVENT_CAPACITY) {
        return 0;
    }
    for (index = 0; index < JADREN_REACTOR_CAPACITY; index += 1) {
        JadrenReactor *reactor = &jadren_reactors[index];
        if (!reactor->active) {
            unsigned int slot;
            HANDLE completion_port = CreateIoCompletionPort(
                (HANDLE)(long long)-1, 0, 0, 0);
            if (completion_port == 0) {
                return 0;
            }
            reactor->active = 1;
            reactor->completion_port = completion_port;
            reactor->max_watches = max_watches;
            reactor->max_events = max_events;
            reactor->event_count = 0;
            reactor->last_error = 0;
            reactor->associated_socket_count = 0;
            for (slot = 0; slot < JADREN_REACTOR_EVENT_CAPACITY; slot += 1) {
                reactor->operations[slot].active = 0;
            }
            for (slot = 0; slot < JADREN_REACTOR_WATCH_CAPACITY; slot += 1) {
                reactor->watches[slot].active = 0;
            }
            jadren_net_reactor_count += 1;
            return (unsigned __int64)index + 1ULL;
        }
    }
    return 0;
}

int net_reactor_watch(unsigned __int64 reactor_token,
                      unsigned __int64 socket_token,
                      unsigned int interest,
                      unsigned __int64 user_data) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    int index;
    if (reactor == 0 || socket_token == 0 || interest == 0 ||
        (interest & ~(JADREN_REACTOR_READABLE | JADREN_REACTOR_WRITABLE |
                      JADREN_REACTOR_ERROR | JADREN_REACTOR_PEER_CLOSED)) != 0) {
        return 0;
    }
    index = jadren_reactor_find_watch(reactor, socket_token);
    if (index < 0) index = jadren_reactor_free_watch(reactor);
    if (index < 0) {
        reactor->last_error = 10055;
        return 0;
    }
    reactor->watches[index].active = 1;
    reactor->watches[index].socket_token = socket_token;
    reactor->watches[index].interest = interest;
    reactor->watches[index].user_data = user_data;
    reactor->last_error = 0;
    return 1;
}

int net_reactor_unwatch(unsigned __int64 reactor_token,
                        unsigned __int64 socket_token) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    int index;
    if (reactor == 0 || socket_token == 0) return 0;
    index = jadren_reactor_find_watch(reactor, socket_token);
    if (index < 0) return 0;
    reactor->watches[index].active = 0;
    reactor->event_count = 0;
    reactor->last_error = 0;
    return 1;
}

static int jadren_reactor_emit_completed_operation(JadrenReactor *reactor) {
    unsigned int index;
    for (index = 0; index < reactor->max_events; index += 1) {
        JadrenReactorOperation *operation = &reactor->operations[index];
        JadrenReactorEvent *event;
        unsigned int flags = 0;
        if (!operation->active || !operation->completed) continue;
        if (operation->timed_out) {
            flags |= JADREN_REACTOR_TIMEOUT;
        } else if (operation->cancel_requested &&
                   operation->completion_error == JADREN_ERROR_OPERATION_ABORTED) {
            flags |= JADREN_REACTOR_CANCELLED;
        } else if (operation->completion_error != 0) {
            flags |= JADREN_REACTOR_ERROR;
        } else if (operation->kind == JADREN_REACTOR_OPERATION_RECEIVE &&
                   operation->buffer_length > 0 && operation->transferred == 0) {
            flags |= JADREN_REACTOR_PEER_CLOSED;
        } else if (operation->kind == JADREN_REACTOR_OPERATION_CONNECT ||
                   operation->kind == JADREN_REACTOR_OPERATION_SEND) {
            flags |= JADREN_REACTOR_WRITABLE;
        } else {
            flags |= JADREN_REACTOR_READABLE;
        }
        if (operation->kind == JADREN_REACTOR_OPERATION_ACCEPT &&
            operation->completion_error == 0 &&
            operation->operation_socket != (JadrenSocket)-1) {
            JadrenSocket listener_handle =
                (JadrenSocket)(operation->socket_token - 1ULL);
            (void)setsockopt(operation->operation_socket, SOL_SOCKET,
                             SO_UPDATE_ACCEPT_CONTEXT,
                             (const char *)&listener_handle,
                             sizeof(listener_handle));
        }
        if (operation->kind == JADREN_REACTOR_OPERATION_CONNECT &&
            operation->completion_error == 0) {
            (void)setsockopt(operation->operation_socket, SOL_SOCKET,
                             SO_UPDATE_CONNECT_CONTEXT, 0, 0);
        }
        event = &reactor->events[reactor->event_count++];
        event->socket_token = operation->result_socket_token != 0
            ? operation->result_socket_token : operation->socket_token;
        event->operation_token = (unsigned __int64)index + 1ULL;
        event->flags = flags;
        event->user_data = operation->user_data;
        event->transferred = operation->transferred;
        operation->active = 0;
        return 1;
    }
    return 0;
}

static JadrenReactorOperation *jadren_reactor_operation_from_overlapped(
    JadrenReactor *reactor, JadrenOverlapped *overlapped) {
    unsigned int index;
    for (index = 0; index < reactor->max_events; index += 1) {
        if (&reactor->operations[index].overlapped == overlapped) {
            if (reactor->operations[index].retired) {
                reactor->operations[index].retired = 0;
                return 0;
            }
            if (!reactor->operations[index].active) continue;
            return &reactor->operations[index];
        }
    }
    return 0;
}

unsigned int net_reactor_poll(unsigned __int64 reactor_token,
                              unsigned int timeout_ms) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    JadrenFdSet read_set;
    JadrenFdSet write_set;
    JadrenFdSet except_set;
    JadrenTimeval timeout;
    int has_watches = 0;
    int has_pending_operations = 0;
    unsigned int index;
    unsigned long long now;
    if (reactor == 0) return 0;
    read_set.count = 0;
    write_set.count = 0;
    except_set.count = 0;
    reactor->event_count = 0;
    reactor->last_error = 0;
    for (index = 0; index < reactor->max_watches; index += 1) {
        if (reactor->watches[index].active) {
            has_watches = 1;
            break;
        }
    }
    now = GetTickCount64();
    for (index = 0; index < reactor->max_events; index += 1) {
        JadrenReactorOperation *operation = &reactor->operations[index];
        if (!operation->active || operation->completed) continue;
        has_pending_operations = 1;
        if (operation->deadline_tick == 0 && timeout_ms > 0) {
            operation->deadline_tick = now + (unsigned long long)timeout_ms;
        }
    }
    if (jadren_reactor_emit_completed_operation(reactor)) {
        return reactor->event_count;
    }
    if (has_pending_operations) {
        unsigned long wait_ms = has_watches ? 0 : timeout_ms;
        unsigned long long deadline = 0;
        unsigned long bytes = 0;
        unsigned long long completion_key = 0;
        JadrenOverlapped *overlapped = 0;
        int completed;
        for (index = 0; index < reactor->max_events; index += 1) {
            JadrenReactorOperation *operation = &reactor->operations[index];
            if (!operation->active || operation->completed ||
                operation->deadline_tick == 0) continue;
            if (deadline == 0 || operation->deadline_tick < deadline) {
                deadline = operation->deadline_tick;
            }
        }
        if (!has_watches && deadline != 0) {
            now = GetTickCount64();
            if (deadline <= now) {
                wait_ms = 0;
            } else if (deadline - now < (unsigned long long)wait_ms) {
                wait_ms = (unsigned long)(deadline - now);
            }
        }
        completed = GetQueuedCompletionStatus(
            reactor->completion_port, &bytes, &completion_key,
            (void **)&overlapped, wait_ms);
        if (overlapped != 0) {
            JadrenReactorOperation *operation =
                jadren_reactor_operation_from_overlapped(reactor, overlapped);
            if (operation != 0) {
                operation->completed = 1;
                operation->transferred = bytes;
                operation->completion_error = completed ? 0 : (int)GetLastError();
                if (operation->kind == JADREN_REACTOR_OPERATION_ACCEPT) {
                    if (operation->completion_error == 0) {
                        JadrenSocket listener_handle =
                            (JadrenSocket)(operation->socket_token - 1ULL);
                        (void)setsockopt(
                            operation->operation_socket, SOL_SOCKET,
                            SO_UPDATE_ACCEPT_CONTEXT,
                            (const char *)&listener_handle,
                            sizeof(listener_handle));
                        operation->result_socket_token =
                            (unsigned __int64)operation->operation_socket + 1ULL;
                    } else if (operation->operation_socket != (JadrenSocket)-1) {
                        closesocket(operation->operation_socket);
                        operation->operation_socket = (JadrenSocket)-1;
                        if (jadren_net_socket_count > 0) {
                            jadren_net_socket_count -= 1;
                        }
                    }
                }
                if (jadren_reactor_emit_completed_operation(reactor)) {
                    return reactor->event_count;
                }
            }
        } else if (!completed) {
            int error = (int)GetLastError();
            if (error != JADREN_WAIT_TIMEOUT) {
                reactor->last_error = error;
                return 0;
            }
        }
        now = GetTickCount64();
        for (index = 0; index < reactor->max_events; index += 1) {
            JadrenReactorOperation *operation = &reactor->operations[index];
            JadrenSocket cancel_socket;
            if (!operation->active || operation->completed ||
                operation->deadline_tick == 0 || now < operation->deadline_tick ||
                operation->timed_out) continue;
            operation->timed_out = 1;
            operation->cancel_requested = 1;
            cancel_socket = operation->kind == JADREN_REACTOR_OPERATION_ACCEPT
                ? (JadrenSocket)(operation->socket_token - 1ULL)
                : operation->operation_socket;
            (void)CancelIoEx((HANDLE)(long long)cancel_socket,
                             &operation->overlapped);
            operation->completed = 1;
            operation->completion_error = 0;
            operation->retired = 1;
            if ((operation->kind == JADREN_REACTOR_OPERATION_ACCEPT ||
                 operation->kind == JADREN_REACTOR_OPERATION_CONNECT) &&
                operation->operation_socket != (JadrenSocket)-1) {
                closesocket(operation->operation_socket);
                operation->operation_socket = (JadrenSocket)-1;
                if (jadren_net_socket_count > 0) {
                    jadren_net_socket_count -= 1;
                }
            }
        }
        if (jadren_reactor_emit_completed_operation(reactor)) {
            return reactor->event_count;
        }
    }
    timeout.seconds = (long)(timeout_ms / 1000U);
    timeout.microseconds = (long)((timeout_ms % 1000U) * 1000U);
    for (index = 0; index < reactor->max_watches; index += 1) {
        JadrenReactorWatch *watch = &reactor->watches[index];
        JadrenSocket socket_handle;
        if (!watch->active || watch->socket_token == 0 ||
            read_set.count >= 64 || write_set.count >= 64 || except_set.count >= 64) {
            continue;
        }
        socket_handle = (JadrenSocket)(watch->socket_token - 1ULL);
        if ((watch->interest & JADREN_REACTOR_READABLE) != 0) {
            read_set.sockets[read_set.count++] = socket_handle;
        }
        if ((watch->interest & JADREN_REACTOR_WRITABLE) != 0) {
            write_set.sockets[write_set.count++] = socket_handle;
        }
        except_set.sockets[except_set.count++] = socket_handle;
    }
    if (!has_watches) return 0;
    {
        int status = select(0, &read_set, &write_set, &except_set, &timeout);
        if (status < 0) {
            reactor->last_error = WSAGetLastError();
            return 0;
        }
        if (status == 0) return 0;
    }
    for (index = 0; index < reactor->max_watches &&
                    reactor->event_count < reactor->max_events; index += 1) {
        JadrenReactorWatch *watch = &reactor->watches[index];
        JadrenSocket socket_handle;
        unsigned int flags = 0;
        if (!watch->active || watch->socket_token == 0) continue;
        socket_handle = (JadrenSocket)(watch->socket_token - 1ULL);
        for (unsigned int cursor = 0; cursor < read_set.count; cursor += 1) {
            if (read_set.sockets[cursor] == socket_handle) {
                flags |= JADREN_REACTOR_READABLE;
                break;
            }
        }
        for (unsigned int cursor = 0; cursor < write_set.count; cursor += 1) {
            if (write_set.sockets[cursor] == socket_handle) {
                flags |= JADREN_REACTOR_WRITABLE;
                break;
            }
        }
        for (unsigned int cursor = 0; cursor < except_set.count; cursor += 1) {
            if (except_set.sockets[cursor] == socket_handle) {
                flags |= JADREN_REACTOR_ERROR;
                break;
            }
        }
        if (flags != 0) {
            JadrenReactorEvent *event = &reactor->events[reactor->event_count++];
            event->socket_token = watch->socket_token;
            event->operation_token = 0;
            event->flags = flags;
            event->user_data = watch->user_data;
            event->transferred = 0;
        }
    }
    return reactor->event_count;
}

unsigned __int64 net_reactor_event_socket(unsigned __int64 reactor_token,
                                          unsigned int index) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    return reactor != 0 && index < reactor->event_count
        ? reactor->events[index].socket_token : 0;
}

unsigned int net_reactor_event_flags(unsigned __int64 reactor_token,
                                     unsigned int index) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    return reactor != 0 && index < reactor->event_count
        ? reactor->events[index].flags : 0;
}

unsigned __int64 net_reactor_event_user(unsigned __int64 reactor_token,
                                        unsigned int index) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    return reactor != 0 && index < reactor->event_count
        ? reactor->events[index].user_data : 0;
}

unsigned __int64 net_reactor_event_operation(unsigned __int64 reactor_token,
                                             unsigned int index) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    return reactor != 0 && index < reactor->event_count
        ? reactor->events[index].operation_token : 0;
}

unsigned __int64 net_reactor_event_bytes(unsigned __int64 reactor_token,
                                         unsigned int index) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    return reactor != 0 && index < reactor->event_count
        ? reactor->events[index].transferred : 0;
}

int net_reactor_error(unsigned __int64 reactor_token) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    return reactor == 0 ? -1 : reactor->last_error;
}

int net_reactor_close(unsigned __int64 reactor_token) {
    JadrenReactor *reactor = jadren_reactor_find(reactor_token);
    unsigned int index;
    if (reactor == 0) return 0;
    for (index = 0; index < reactor->max_events; index += 1) {
        JadrenReactorOperation *operation = &reactor->operations[index];
        JadrenSocket cancel_socket;
        if (!operation->active) continue;
        cancel_socket = operation->kind == JADREN_REACTOR_OPERATION_ACCEPT
            ? (JadrenSocket)(operation->socket_token - 1ULL)
            : operation->operation_socket;
        if (!operation->completed) {
            (void)CancelIoEx((HANDLE)(long long)cancel_socket, &operation->overlapped);
        }
        if ((operation->kind == JADREN_REACTOR_OPERATION_ACCEPT ||
             operation->kind == JADREN_REACTOR_OPERATION_CONNECT) &&
            operation->operation_socket != (JadrenSocket)-1) {
            closesocket(operation->operation_socket);
            if (jadren_net_socket_count > 0) {
                jadren_net_socket_count -= 1;
            }
        }
        operation->active = 0;
    }
    if (reactor->completion_port != 0) {
        CloseHandle(reactor->completion_port);
        reactor->completion_port = 0;
    }
    reactor->active = 0;
    if (jadren_net_reactor_count > 0) {
        jadren_net_reactor_count -= 1;
    }
    reactor->event_count = 0;
    reactor->last_error = 0;
    jadren_net_maybe_cleanup();
    return 1;
}
#endif

#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
#define JADREN_TLS_CAPACITY 8
#define JADREN_TLS_SERVER_TOKEN_BASE JADREN_TLS_CAPACITY
#define JADREN_TLS_IO_CAPACITY 32768
#define JADREN_TLS_STATE_CLOSED 0U
#define JADREN_TLS_STATE_HANDSHAKING 1U
#define JADREN_TLS_STATE_OPEN 2U
#define JADREN_TLS_STATE_ERROR 3U
#define JADREN_TLS_STATE_PEER_CLOSED 4U
#define JADREN_SEC_E_OK 0
#define JADREN_SEC_I_CONTINUE_NEEDED 0x00090312
#define JADREN_SEC_I_COMPLETE_NEEDED 0x00090313
#define JADREN_SEC_I_COMPLETE_AND_CONTINUE 0x00090314
#define JADREN_SEC_E_INCOMPLETE_MESSAGE ((int)0x80190318)
#define JADREN_SEC_I_RENEGOTIATE 0x00090321
#define JADREN_SEC_I_CONTEXT_EXPIRED 0x00090317
#define JADREN_SECPKG_CRED_OUTBOUND 2U
#define JADREN_SECPKG_CRED_INBOUND 1U
#define JADREN_SCHANNEL_CRED_VERSION 4U
#define JADREN_SCH_USE_STRONG_CRYPTO 0x00400000U
#define JADREN_SCH_CRED_NO_DEFAULT_CREDS 0x00000010U
#define JADREN_SCH_CRED_MANUAL_CRED_VALIDATION 0x00000008U
#define JADREN_SCH_CRED_IGNORE_NO_REVOCATION_CHECK 0x00000800U
#define JADREN_SCH_CRED_IGNORE_REVOCATION_OFFLINE 0x00001000U
#define JADREN_SCH_CRED_IGNORE_CERT_WRONG_USAGE 0x00002000U
#define JADREN_ISC_REQ_REPLAY_DETECT 0x00000004U
#define JADREN_ISC_REQ_SEQUENCE_DETECT 0x00000008U
#define JADREN_ISC_REQ_CONFIDENTIALITY 0x00000010U
#define JADREN_ISC_REQ_EXTENDED_ERROR 0x00004000U
#define JADREN_ISC_REQ_ALLOCATE_MEMORY 0x00000100U
#define JADREN_ISC_REQ_STREAM 0x00008000U
#define JADREN_ASC_REQ_REPLAY_DETECT 0x00000004U
#define JADREN_ASC_REQ_SEQUENCE_DETECT 0x00000008U
#define JADREN_ASC_REQ_CONFIDENTIALITY 0x00000010U
#define JADREN_ASC_REQ_EXTENDED_ERROR 0x00000080U
#define JADREN_ASC_REQ_ALLOCATE_MEMORY 0x00000100U
#define JADREN_ASC_REQ_STREAM 0x00008000U
#define JADREN_SECURITY_NATIVE_DREP 0x00000010U
#define JADREN_SECBUFFER_EMPTY 0U
#define JADREN_SECBUFFER_DATA 1U
#define JADREN_SECBUFFER_TOKEN 2U
#define JADREN_SECBUFFER_EXTRA 5U
#define JADREN_SECBUFFER_STREAM_TRAILER 6U
#define JADREN_SECBUFFER_STREAM_HEADER 7U
#define JADREN_SECBUFFER_VERSION 0U
#define JADREN_SECPKG_ATTR_STREAM_SIZES 4U

typedef struct JadrenTlsClient {
    int active;
    int credential_ready;
    int context_ready;
    unsigned int state;
    int last_error;
    unsigned __int64 socket_token;
    int verify_peer;
    JadrenSecHandle credential;
    JadrenSecHandle context;
    wchar_t server_name[256];
    unsigned char input[JADREN_TLS_IO_CAPACITY];
    unsigned long input_length;
    unsigned char plain[JADREN_TLS_IO_CAPACITY];
    unsigned long plain_length;
    unsigned long plain_offset;
    JadrenStreamSizes sizes;
} JadrenTlsClient;

typedef struct JadrenTlsServer {
    int active;
    int credential_ready;
    int context_ready;
    unsigned int state;
    int last_error;
    unsigned __int64 socket_token;
    JadrenSecHandle credential;
    JadrenSecHandle context;
    JadrenCertStore certificate_store;
    JadrenCertContext *certificate;
    unsigned char input[JADREN_TLS_IO_CAPACITY];
    unsigned long input_length;
    unsigned char plain[JADREN_TLS_IO_CAPACITY];
    unsigned long plain_length;
    unsigned long plain_offset;
    JadrenStreamSizes sizes;
} JadrenTlsServer;

static JadrenTlsClient jadren_tls_clients[JADREN_TLS_CAPACITY];
static JadrenTlsServer jadren_tls_servers[JADREN_TLS_CAPACITY];
static unsigned char jadren_tls_pfx_data[65536];

static void jadren_tls_zero(void *data, unsigned long length) {
    unsigned char *bytes = (unsigned char *)data;
    unsigned long index;
    if (bytes == 0) return;
    for (index = 0; index < length; index += 1) bytes[index] = 0;
}

static JadrenTlsClient *jadren_tls_find(unsigned __int64 token) {
    if (token == 0 || token > JADREN_TLS_CAPACITY) return 0;
    return &jadren_tls_clients[token - 1ULL];
}

static JadrenTlsServer *jadren_tls_find_server(unsigned __int64 token) {
    if (token <= JADREN_TLS_SERVER_TOKEN_BASE ||
        token > JADREN_TLS_SERVER_TOKEN_BASE + JADREN_TLS_CAPACITY) return 0;
    return &jadren_tls_servers[token - JADREN_TLS_SERVER_TOKEN_BASE - 1ULL];
}

static int jadren_tls_read_file(const char *path_data,
                                unsigned __int64 path_length,
                                unsigned char *output_data,
                                unsigned long output_capacity,
                                unsigned long *output_length) {
    wchar_t path[1024];
    JadrenLargeInteger file_size;
    HANDLE handle;
    DWORD received = 0;
    if (output_length == 0 || output_data == 0 || output_capacity == 0 ||
        !path_to_wide(path_data, path_length, path, 1024)) return 0;
    handle = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING,
                         FILE_ATTRIBUTE_NORMAL, 0);
    if (handle == INVALID_HANDLE_VALUE || !GetFileSizeEx(handle, &file_size) ||
        file_size.QuadPart <= 0 ||
        (unsigned __int64)file_size.QuadPart > output_capacity ||
        !ReadFile(handle, output_data, (DWORD)file_size.QuadPart, &received, 0)) {
        if (handle != INVALID_HANDLE_VALUE) CloseHandle(handle);
        return 0;
    }
    CloseHandle(handle);
    if (received != (DWORD)file_size.QuadPart) return 0;
    *output_length = received;
    return 1;
}

static int jadren_tls_send_raw(JadrenTlsClient *client,
                               const unsigned char *data,
                               unsigned long length) {
    unsigned long offset = 0;
    while (offset < length) {
        unsigned long chunk = length - offset;
        int sent;
        if (chunk > 0x7FFFFFFFUL) chunk = 0x7FFFFFFFUL;
        sent = send((JadrenSocket)(client->socket_token - 1ULL),
                    (const char *)(data + offset), (int)chunk, 0);
        if (sent <= 0) return 0;
        offset += (unsigned long)sent;
    }
    return 1;
}

static int jadren_tls_receive_raw(JadrenTlsClient *client,
                                  unsigned char *data,
                                  unsigned long capacity) {
    int received;
    int error;
    if (capacity == 0) return 0;
    received = recv((JadrenSocket)(client->socket_token - 1ULL),
                    (char *)data, (int)capacity, 0);
    if (received > 0) return received;
    if (received == 0) return 0;
    error = WSAGetLastError();
    return error == 10004 || error == 10035 || error == 10060 ? -1 : 0;
}

static int jadren_tls_read_more(JadrenTlsClient *client) {
    int received;
    if (client->input_length >= JADREN_TLS_IO_CAPACITY) return 0;
    received = jadren_tls_receive_raw(
        client, client->input + client->input_length,
        JADREN_TLS_IO_CAPACITY - client->input_length);
    if (received <= 0) return received;
    client->input_length += (unsigned long)received;
    return received;
}

static unsigned int jadren_tls_step_impl(JadrenTlsClient *client,
                                         unsigned int timeout_ms) {
    JadrenSecBuffer input_buffers[2];
    JadrenSecBufferDesc input_desc;
    JadrenSecBuffer output_buffer;
    JadrenSecBufferDesc output_desc;
    unsigned char output_data[JADREN_TLS_IO_CAPACITY];
    unsigned long attributes = 0;
    unsigned long output_length;
    int status;
    int received;
    if (client == 0 || !client->active) return 0;
    if (client->state != JADREN_TLS_STATE_HANDSHAKING) return client->state;
    (void)net_socket_set_timeout(client->socket_token,
                                 timeout_ms == 0 ? 1 : timeout_ms);
    jadren_tls_zero(input_buffers, sizeof(input_buffers));
    jadren_tls_zero(output_data, sizeof(output_data));
    output_buffer.length = sizeof(output_data);
    output_buffer.type = JADREN_SECBUFFER_TOKEN;
    output_buffer.data = output_data;
    output_desc.version = JADREN_SECBUFFER_VERSION;
    output_desc.count = 1;
    output_desc.buffers = &output_buffer;
    if (client->input_length > 0) {
        input_buffers[0].length = client->input_length;
        input_buffers[0].type = JADREN_SECBUFFER_TOKEN;
        input_buffers[0].data = client->input;
        input_buffers[1].length = 0;
        input_buffers[1].type = JADREN_SECBUFFER_EMPTY;
        input_buffers[1].data = 0;
        input_desc.version = JADREN_SECBUFFER_VERSION;
        input_desc.count = 2;
        input_desc.buffers = input_buffers;
    }
    status = InitializeSecurityContextW(
        &client->credential, client->context_ready ? &client->context : 0,
        client->server_name,
        JADREN_ISC_REQ_REPLAY_DETECT | JADREN_ISC_REQ_SEQUENCE_DETECT |
            JADREN_ISC_REQ_CONFIDENTIALITY | JADREN_ISC_REQ_EXTENDED_ERROR |
            JADREN_ISC_REQ_STREAM,
        0, JADREN_SECURITY_NATIVE_DREP,
        client->input_length > 0 ? &input_desc : 0, 0, &client->context,
        &output_desc, &attributes, 0);
    if (status == JADREN_SEC_E_INCOMPLETE_MESSAGE) {
        received = jadren_tls_read_more(client);
        if (received == 0) {
            client->state = JADREN_TLS_STATE_PEER_CLOSED;
            return 3;
        }
        return 1;
    }
    if (status != JADREN_SEC_E_OK && status != JADREN_SEC_I_CONTINUE_NEEDED) {
        client->last_error = status;
        client->state = JADREN_TLS_STATE_ERROR;
        return 4;
    }
    client->context_ready = 1;
    output_length = output_buffer.length;
    if (output_length > 0 && !jadren_tls_send_raw(client, output_data, output_length)) {
        client->last_error = -1;
        client->state = JADREN_TLS_STATE_ERROR;
        return 4;
    }
    if (input_buffers[1].type == JADREN_SECBUFFER_EXTRA &&
        input_buffers[1].length <= client->input_length) {
        unsigned long extra = input_buffers[1].length;
        unsigned long source = client->input_length - extra;
        unsigned long index;
        for (index = 0; index < extra; index += 1) {
            client->input[index] = client->input[source + index];
        }
        client->input_length = extra;
    } else {
        client->input_length = 0;
    }
    if (status == JADREN_SEC_E_OK) {
        if (QueryContextAttributesW(&client->context,
                                    JADREN_SECPKG_ATTR_STREAM_SIZES,
                                    &client->sizes) != JADREN_SEC_E_OK ||
            client->sizes.header + client->sizes.trailer >= JADREN_TLS_IO_CAPACITY) {
            client->last_error = -2;
            client->state = JADREN_TLS_STATE_ERROR;
            return 4;
        }
        client->state = JADREN_TLS_STATE_OPEN;
        client->last_error = 0;
        return 2;
    }
    if (client->input_length == 0) {
        received = jadren_tls_read_more(client);
        if (received == 0) {
            client->state = JADREN_TLS_STATE_PEER_CLOSED;
            return 3;
        }
    }
    return 1;
}

unsigned __int64 net_tls_open_client(unsigned __int64 socket_token,
                                     const char *server_data,
                                     unsigned __int64 server_length,
                                     int verify_peer) {
    wchar_t server_name[256];
    wchar_t package_name[] = {
        'M','i','c','r','o','s','o','f','t',' ','U','n','i','f','i','e','d',' ',
        'S','e','c','u','r','i','t','y',' ','P','r','o','t','o','c','o','l',' ',
        'P','r','o','v','i','d','e','r',0
    };
    JadrenSchannelCred credentials;
    JadrenTimeStamp expiry;
    int status;
    int index;
    if (socket_token == 0 || server_data == 0 || server_length == 0 ||
        server_length >= 256 || !path_to_wide(server_data, server_length,
                                               server_name, 256)) {
        return 0;
    }
    for (index = 0; index < JADREN_TLS_CAPACITY; index += 1) {
        if (!jadren_tls_clients[index].active) break;
    }
    if (index == JADREN_TLS_CAPACITY) return 0;
    jadren_tls_zero(&jadren_tls_clients[index], sizeof(jadren_tls_clients[index]));
    jadren_tls_clients[index].active = 1;
    jadren_tls_clients[index].state = JADREN_TLS_STATE_HANDSHAKING;
    jadren_tls_clients[index].socket_token = socket_token;
    jadren_tls_clients[index].verify_peer = verify_peer != 0;
    jadren_tls_zero(&credentials, sizeof(credentials));
    credentials.version = JADREN_SCHANNEL_CRED_VERSION;
    credentials.flags = JADREN_SCH_USE_STRONG_CRYPTO | JADREN_SCH_CRED_NO_DEFAULT_CREDS;
    if (!verify_peer) {
        credentials.flags |= JADREN_SCH_CRED_MANUAL_CRED_VALIDATION |
                             JADREN_SCH_CRED_IGNORE_NO_REVOCATION_CHECK |
                             JADREN_SCH_CRED_IGNORE_REVOCATION_OFFLINE |
                             JADREN_SCH_CRED_IGNORE_CERT_WRONG_USAGE;
    }
    status = AcquireCredentialsHandleW(0, package_name, JADREN_SECPKG_CRED_OUTBOUND,
                                        0, &credentials, 0, 0,
                                        &jadren_tls_clients[index].credential,
                                        &expiry);
    if (status != JADREN_SEC_E_OK) {
        jadren_tls_clients[index].last_error = status;
        jadren_tls_clients[index].state = JADREN_TLS_STATE_ERROR;
        return (unsigned __int64)index + 1ULL;
    }
    jadren_tls_clients[index].credential_ready = 1;
    jadren_tls_zero(&jadren_tls_clients[index].context,
                    sizeof(jadren_tls_clients[index].context));
    jadren_tls_zero(jadren_tls_clients[index].server_name,
                    sizeof(jadren_tls_clients[index].server_name));
    for (status = 0; status < 256; status += 1) {
        jadren_tls_clients[index].server_name[status] = server_name[status];
        if (server_name[status] == 0) break;
    }
    jadren_tls_clients[index].context_ready = 0;
    jadren_tls_clients[index].state = JADREN_TLS_STATE_HANDSHAKING;
    jadren_tls_clients[index].last_error = 0;
    jadren_tls_clients[index].input_length = 0;
    jadren_tls_clients[index].plain_length = 0;
    jadren_tls_clients[index].plain_offset = 0;
    return (unsigned __int64)index + 1ULL;
}

unsigned __int64 net_tls_open_server(unsigned __int64 socket_token,
                                     const char *certificate_data,
                                     unsigned __int64 certificate_length,
                                     const char *private_key_data,
                                     unsigned __int64 private_key_length) {
    unsigned long pfx_length = 0;
    wchar_t password[256];
    wchar_t package_name[] = {
        'M','i','c','r','o','s','o','f','t',' ','U','n','i','f','i','e','d',' ',
        'S','e','c','u','r','i','t','y',' ','P','r','o','t','o','c','o','l',' ',
        'P','r','o','v','i','d','e','r',0
    };
    JadrenCryptDataBlob blob;
    JadrenSchannelCred credentials;
    JadrenTimeStamp expiry;
    JadrenCertStore store;
    JadrenCertContext *certificate;
    JadrenCertContext *certificates[1];
    int status;
    int index;
    if (socket_token == 0 || certificate_data == 0 || certificate_length == 0 ||
        private_key_data == 0 || private_key_length >= 256 ||
        !jadren_tls_read_file(certificate_data, certificate_length,
                              jadren_tls_pfx_data, sizeof(jadren_tls_pfx_data),
                              &pfx_length)) return 0;
    password[0] = 0;
    if (private_key_length > 0 &&
        !path_to_wide(private_key_data, private_key_length, password,
                      sizeof(password) / sizeof(password[0]))) return 0;
    for (index = 0; index < JADREN_TLS_CAPACITY; index += 1) {
        if (!jadren_tls_servers[index].active) break;
    }
    if (index == JADREN_TLS_CAPACITY) return 0;
    jadren_tls_zero(&jadren_tls_servers[index], sizeof(jadren_tls_servers[index]));
    blob.length = pfx_length;
    blob.data = jadren_tls_pfx_data;
    store = PFXImportCertStore(&blob, password, 0);
    if (store == 0) return 0;
    certificate = CertEnumCertificatesInStore(store, 0);
    if (certificate == 0) {
        (void)CertCloseStore(store, 0);
        return 0;
    }
    jadren_tls_zero(&credentials, sizeof(credentials));
    credentials.version = JADREN_SCHANNEL_CRED_VERSION;
    credentials.credential_count = 1;
    certificates[0] = certificate;
    credentials.certificates = certificates;
    credentials.flags = JADREN_SCH_USE_STRONG_CRYPTO | JADREN_SCH_CRED_NO_DEFAULT_CREDS;
    status = AcquireCredentialsHandleW(0, package_name, JADREN_SECPKG_CRED_INBOUND,
                                        0, &credentials, 0, 0,
                                        &jadren_tls_servers[index].credential,
                                        &expiry);
    if (status != JADREN_SEC_E_OK) {
        (void)CertCloseStore(store, 0);
        return 0;
    }
    jadren_tls_servers[index].active = 1;
    jadren_tls_servers[index].credential_ready = 1;
    jadren_tls_servers[index].state = JADREN_TLS_STATE_HANDSHAKING;
    jadren_tls_servers[index].socket_token = socket_token;
    jadren_tls_servers[index].certificate_store = store;
    jadren_tls_servers[index].certificate = certificate;
    return JADREN_TLS_SERVER_TOKEN_BASE + (unsigned __int64)index + 1ULL;
}

/* Open a TLS server from caller-owned byte views. The explicit lengths are
 * checked against the hidden slice capacities before reusing the legacy
 * platform credential loader (PFX path + password on Windows, PEM paths on
 * POSIX). */
unsigned __int64 net_tls_open_server_paths(unsigned __int64 socket_token,
                                           const unsigned char *certificate_data,
                                           unsigned __int64 certificate_capacity,
                                           unsigned __int64 certificate_length,
                                           const unsigned char *private_key_data,
                                           unsigned __int64 private_key_capacity,
                                           unsigned __int64 private_key_length) {
    if (certificate_length > certificate_capacity ||
        private_key_length > private_key_capacity ||
        (certificate_data == 0 && certificate_length > 0) ||
        (private_key_data == 0 && private_key_length > 0)) {
        return 0;
    }
    return net_tls_open_server(socket_token,
                               (const char *)certificate_data,
                               certificate_length,
                               (const char *)private_key_data,
                               private_key_length);
}

static int jadren_tls_receive_raw_server(JadrenTlsServer *server,
                                         unsigned char *data,
                                         unsigned long capacity) {
    int received;
    int error;
    if (capacity == 0) return 0;
    received = recv((JadrenSocket)(server->socket_token - 1ULL),
                    (char *)data, (int)capacity, 0);
    if (received > 0) return received;
    if (received == 0) return 0;
    error = WSAGetLastError();
    return error == 10004 || error == 10035 || error == 10060 ? -1 : 0;
}

static int jadren_tls_send_raw_server(JadrenTlsServer *server,
                                      const unsigned char *data,
                                      unsigned long length) {
    unsigned long offset = 0;
    while (offset < length) {
        unsigned long chunk = length - offset;
        int sent;
        if (chunk > 0x7FFFFFFFUL) chunk = 0x7FFFFFFFUL;
        sent = send((JadrenSocket)(server->socket_token - 1ULL),
                    (const char *)(data + offset), (int)chunk, 0);
        if (sent <= 0) return 0;
        offset += (unsigned long)sent;
    }
    return 1;
}

static int jadren_tls_read_more_server(JadrenTlsServer *server) {
    int received;
    if (server->input_length >= JADREN_TLS_IO_CAPACITY) return 0;
    received = jadren_tls_receive_raw_server(
        server, server->input + server->input_length,
        JADREN_TLS_IO_CAPACITY - server->input_length);
    if (received <= 0) return received;
    server->input_length += (unsigned long)received;
    return received;
}

static unsigned int jadren_tls_step_server_impl(JadrenTlsServer *server,
                                                unsigned int timeout_ms) {
    JadrenSecBuffer input_buffers[2];
    JadrenSecBufferDesc input_desc;
    JadrenSecBuffer output_buffer;
    JadrenSecBufferDesc output_desc;
    unsigned char output_data[JADREN_TLS_IO_CAPACITY];
    unsigned long attributes = 0;
    unsigned long output_length;
    int status;
    int received;
    if (server == 0 || !server->active) return 0;
    if (server->state != JADREN_TLS_STATE_HANDSHAKING) return server->state;
    (void)net_socket_set_timeout(server->socket_token,
                                 timeout_ms == 0 ? 1 : timeout_ms);
    if (server->input_length == 0) {
        received = jadren_tls_read_more_server(server);
        if (received == 0) {
            server->state = JADREN_TLS_STATE_PEER_CLOSED;
            return 3;
        }
        if (received < 0) return 1;
    }
    jadren_tls_zero(input_buffers, sizeof(input_buffers));
    jadren_tls_zero(output_data, sizeof(output_data));
    input_buffers[0].length = server->input_length;
    input_buffers[0].type = JADREN_SECBUFFER_TOKEN;
    input_buffers[0].data = server->input;
    input_buffers[1].type = JADREN_SECBUFFER_EMPTY;
    input_desc.version = JADREN_SECBUFFER_VERSION;
    input_desc.count = 2;
    input_desc.buffers = input_buffers;
    output_buffer.length = sizeof(output_data);
    output_buffer.type = JADREN_SECBUFFER_TOKEN;
    output_buffer.data = output_data;
    output_desc.version = JADREN_SECBUFFER_VERSION;
    output_desc.count = 1;
    output_desc.buffers = &output_buffer;
    status = AcceptSecurityContext(
        &server->credential, server->context_ready ? &server->context : 0,
        &input_desc,
        JADREN_ASC_REQ_REPLAY_DETECT | JADREN_ASC_REQ_SEQUENCE_DETECT |
            JADREN_ASC_REQ_CONFIDENTIALITY | JADREN_ASC_REQ_EXTENDED_ERROR |
            JADREN_ASC_REQ_STREAM,
        JADREN_SECURITY_NATIVE_DREP, &server->context, &output_desc,
        &attributes, 0);
    if (status == JADREN_SEC_E_INCOMPLETE_MESSAGE) {
        received = jadren_tls_read_more_server(server);
        if (received == 0) {
            server->state = JADREN_TLS_STATE_PEER_CLOSED;
            return 3;
        }
        return 1;
    }
    if (status == JADREN_SEC_I_COMPLETE_NEEDED ||
        status == JADREN_SEC_I_COMPLETE_AND_CONTINUE) {
        if (CompleteAuthToken(&server->context, &output_desc) != JADREN_SEC_E_OK) {
            server->last_error = -5;
            server->state = JADREN_TLS_STATE_ERROR;
            return 4;
        }
    } else if (status != JADREN_SEC_E_OK && status != JADREN_SEC_I_CONTINUE_NEEDED) {
        server->last_error = status;
        server->state = JADREN_TLS_STATE_ERROR;
        return 4;
    }
    server->context_ready = 1;
    output_length = output_buffer.length;
    if (output_length > 0 &&
        !jadren_tls_send_raw_server(server, output_data, output_length)) {
        server->last_error = -1;
        server->state = JADREN_TLS_STATE_ERROR;
        return 4;
    }
    if (input_buffers[1].type == JADREN_SECBUFFER_EXTRA &&
        input_buffers[1].length <= server->input_length) {
        unsigned long extra = input_buffers[1].length;
        unsigned long source = server->input_length - extra;
        unsigned long index;
        for (index = 0; index < extra; index += 1) {
            server->input[index] = server->input[source + index];
        }
        server->input_length = extra;
    } else {
        server->input_length = 0;
    }
    if (status == JADREN_SEC_E_OK || status == JADREN_SEC_I_COMPLETE_NEEDED) {
        if (QueryContextAttributesW(&server->context,
                                    JADREN_SECPKG_ATTR_STREAM_SIZES,
                                    &server->sizes) != JADREN_SEC_E_OK ||
            server->sizes.header + server->sizes.trailer >= JADREN_TLS_IO_CAPACITY) {
            server->last_error = -2;
            server->state = JADREN_TLS_STATE_ERROR;
            return 4;
        }
        server->state = JADREN_TLS_STATE_OPEN;
        server->last_error = 0;
        return 2;
    }
    if (server->input_length == 0) {
        received = jadren_tls_read_more_server(server);
        if (received == 0) {
            server->state = JADREN_TLS_STATE_PEER_CLOSED;
            return 3;
        }
    }
    return 1;
}

unsigned int net_tls_step(unsigned __int64 token, unsigned int timeout_ms) {
    JadrenTlsServer *server = jadren_tls_find_server(token);
    if (server != 0) return jadren_tls_step_server_impl(server, timeout_ms);
    return jadren_tls_step_impl(jadren_tls_find(token), timeout_ms);
}

unsigned int net_tls_state(unsigned __int64 token) {
    JadrenTlsClient *client = jadren_tls_find(token);
    JadrenTlsServer *server = jadren_tls_find_server(token);
    if (server != 0) return !server->active ? JADREN_TLS_STATE_CLOSED : server->state;
    return client == 0 || !client->active ? JADREN_TLS_STATE_CLOSED : client->state;
}

int net_tls_error(unsigned __int64 token) {
    JadrenTlsClient *client = jadren_tls_find(token);
    JadrenTlsServer *server = jadren_tls_find_server(token);
    if (server != 0) return !server->active ? -1 : server->last_error;
    return client == 0 || !client->active ? -1 : client->last_error;
}

unsigned __int64 net_tls_send(unsigned __int64 token,
                              const unsigned char *input_data,
                              unsigned __int64 input_length) {
    JadrenTlsClient *client = jadren_tls_find(token);
    JadrenTlsServer *server = jadren_tls_find_server(token);
    unsigned char record[JADREN_TLS_IO_CAPACITY];
    JadrenSecBuffer buffers[4];
    JadrenSecBufferDesc message;
    unsigned long header;
    unsigned long trailer;
    unsigned long payload;
    unsigned long index;
    int status;
    if (server != 0) {
        if (!server->active || server->state != JADREN_TLS_STATE_OPEN ||
            (input_data == 0 && input_length > 0) || input_length == 0 ||
            input_length > server->sizes.maximum_message ||
            input_length > 0xFFFFFFFFULL) return 0;
        header = server->sizes.header;
        trailer = server->sizes.trailer;
        payload = (unsigned long)input_length;
        if (header > sizeof(record) || payload > sizeof(record) - header ||
            trailer > sizeof(record) - header - payload) return 0;
        for (index = 0; index < payload; index += 1) {
            record[header + index] = input_data[index];
        }
        buffers[0].length = header;
        buffers[0].type = JADREN_SECBUFFER_STREAM_HEADER;
        buffers[0].data = record;
        buffers[1].length = payload;
        buffers[1].type = JADREN_SECBUFFER_DATA;
        buffers[1].data = record + header;
        buffers[2].length = trailer;
        buffers[2].type = JADREN_SECBUFFER_STREAM_TRAILER;
        buffers[2].data = record + header + payload;
        buffers[3].length = 0;
        buffers[3].type = JADREN_SECBUFFER_EMPTY;
        buffers[3].data = 0;
        message.version = JADREN_SECBUFFER_VERSION;
        message.count = 4;
        message.buffers = buffers;
        status = EncryptMessage(&server->context, 0, &message, 0);
        if (status != JADREN_SEC_E_OK ||
            !jadren_tls_send_raw_server(server, record,
                                        buffers[0].length + buffers[1].length +
                                            buffers[2].length)) {
            server->last_error = status == JADREN_SEC_E_OK ? -3 : status;
            server->state = JADREN_TLS_STATE_ERROR;
            return 0;
        }
        return input_length;
    }
    if (client == 0 || !client->active || client->state != JADREN_TLS_STATE_OPEN ||
        (input_data == 0 && input_length > 0) || input_length == 0 ||
        input_length > client->sizes.maximum_message || input_length > 0xFFFFFFFFULL) {
        return 0;
    }
    header = client->sizes.header;
    trailer = client->sizes.trailer;
    payload = (unsigned long)input_length;
    if (header > sizeof(record) || payload > sizeof(record) - header ||
        trailer > sizeof(record) - header - payload) return 0;
    for (index = 0; index < payload; index += 1) record[header + index] = input_data[index];
    buffers[0].length = header;
    buffers[0].type = JADREN_SECBUFFER_STREAM_HEADER;
    buffers[0].data = record;
    buffers[1].length = payload;
    buffers[1].type = JADREN_SECBUFFER_DATA;
    buffers[1].data = record + header;
    buffers[2].length = trailer;
    buffers[2].type = JADREN_SECBUFFER_STREAM_TRAILER;
    buffers[2].data = record + header + payload;
    buffers[3].length = 0;
    buffers[3].type = JADREN_SECBUFFER_EMPTY;
    buffers[3].data = 0;
    message.version = JADREN_SECBUFFER_VERSION;
    message.count = 4;
    message.buffers = buffers;
    status = EncryptMessage(&client->context, 0, &message, 0);
    if (status != JADREN_SEC_E_OK ||
        !jadren_tls_send_raw(client, record,
                             buffers[0].length + buffers[1].length + buffers[2].length)) {
        client->last_error = status == JADREN_SEC_E_OK ? -3 : status;
        client->state = JADREN_TLS_STATE_ERROR;
        return 0;
    }
    return input_length;
}

/* Send only the explicit prefix of a caller-owned slice. The hidden slice
 * capacity is checked before delegating to the regular TLS record sender. */
unsigned __int64 net_tls_send_prefix(unsigned __int64 token,
                                     const unsigned char *input_data,
                                     unsigned __int64 input_capacity,
                                     unsigned __int64 send_length) {
    if (send_length > input_capacity) return 0;
    return net_tls_send(token, input_data, send_length);
}

/* Send the complete explicit prefix, splitting it into native TLS records
 * when the caller-owned payload exceeds the platform maximum message size. */
unsigned __int64 net_tls_send_all_prefix(unsigned __int64 token,
                                         const unsigned char *input_data,
                                         unsigned __int64 input_capacity,
                                         unsigned __int64 send_length) {
    unsigned __int64 offset = 0;
    if (send_length > input_capacity ||
        (input_data == 0 && send_length > 0)) return 0;
    while (offset < send_length) {
        unsigned __int64 chunk = send_length - offset;
        if (chunk > 16384ULL) chunk = 16384ULL;
        unsigned __int64 sent = net_tls_send(token, input_data + offset, chunk);
        if (sent == 0 || sent > chunk) return offset;
        offset += sent;
    }
    return offset;
}

unsigned __int64 net_tls_receive(unsigned __int64 token,
                                 unsigned char *output_data,
                                 unsigned __int64 output_length) {
    JadrenTlsClient *client = jadren_tls_find(token);
    JadrenTlsServer *server = jadren_tls_find_server(token);
    JadrenSecBuffer buffers[4];
    JadrenSecBufferDesc message;
    unsigned long quality = 0;
    unsigned long index;
    unsigned long plain_length;
    int status;
    int received;
    if (server != 0) {
        if (!server->active || server->state != JADREN_TLS_STATE_OPEN ||
            output_data == 0 || output_length == 0) return 0;
        if (server->plain_offset < server->plain_length) {
            plain_length = server->plain_length - server->plain_offset;
            if (plain_length > output_length) plain_length = (unsigned long)output_length;
            for (index = 0; index < plain_length; index += 1) {
                output_data[index] = server->plain[server->plain_offset + index];
            }
            server->plain_offset += plain_length;
            if (server->plain_offset == server->plain_length) {
                server->plain_offset = 0;
                server->plain_length = 0;
            }
            return plain_length;
        }
        if (server->input_length == 0) {
            received = jadren_tls_read_more_server(server);
            if (received == 0) {
                server->state = JADREN_TLS_STATE_PEER_CLOSED;
                return 0;
            }
            if (received < 0) return 0;
        }
        for (;;) {
            buffers[0].length = server->input_length;
            buffers[0].type = JADREN_SECBUFFER_DATA;
            buffers[0].data = server->input;
            buffers[1].length = 0; buffers[1].type = JADREN_SECBUFFER_EMPTY; buffers[1].data = 0;
            buffers[2].length = 0; buffers[2].type = JADREN_SECBUFFER_EMPTY; buffers[2].data = 0;
            buffers[3].length = 0; buffers[3].type = JADREN_SECBUFFER_EMPTY; buffers[3].data = 0;
            message.version = JADREN_SECBUFFER_VERSION;
            message.count = 4;
            message.buffers = buffers;
            status = DecryptMessage(&server->context, &message, 0, &quality);
            if (status == JADREN_SEC_E_INCOMPLETE_MESSAGE) {
                received = jadren_tls_read_more_server(server);
                if (received == 0) {
                    server->state = JADREN_TLS_STATE_PEER_CLOSED;
                    return 0;
                }
                if (received < 0) return 0;
                continue;
            }
            if (status == JADREN_SEC_I_CONTEXT_EXPIRED) {
                server->state = JADREN_TLS_STATE_PEER_CLOSED;
                server->last_error = status;
                return 0;
            }
            if (status != JADREN_SEC_E_OK) {
                server->state = JADREN_TLS_STATE_ERROR;
                server->last_error = status;
                return 0;
            }
            {
                unsigned long data_length = 0;
                unsigned long extra_length = 0;
                unsigned char *data = 0;
                unsigned char *extra = 0;
                for (index = 0; index < 4; index += 1) {
                    if (buffers[index].type == JADREN_SECBUFFER_DATA) {
                        data = (unsigned char *)buffers[index].data;
                        data_length = buffers[index].length;
                    } else if (buffers[index].type == JADREN_SECBUFFER_EXTRA) {
                        extra = (unsigned char *)buffers[index].data;
                        extra_length = buffers[index].length;
                    }
                }
                if (data == 0 || data_length > sizeof(server->plain)) {
                    server->last_error = -4;
                    server->state = JADREN_TLS_STATE_ERROR;
                    return 0;
                }
                for (index = 0; index < data_length; index += 1) server->plain[index] = data[index];
                server->plain_length = data_length;
                server->plain_offset = 0;
                if (extra != 0 && extra_length <= server->input_length) {
                    unsigned long source = server->input_length - extra_length;
                    for (index = 0; index < extra_length; index += 1) server->input[index] = extra[index];
                    server->input_length = extra_length;
                } else {
                    server->input_length = 0;
                }
                return net_tls_receive(token, output_data, output_length);
            }
        }
    }
    if (client == 0 || !client->active || client->state != JADREN_TLS_STATE_OPEN ||
        output_data == 0 || output_length == 0) return 0;
    if (client->plain_offset < client->plain_length) {
        plain_length = client->plain_length - client->plain_offset;
        if (plain_length > output_length) plain_length = (unsigned long)output_length;
        for (index = 0; index < plain_length; index += 1) {
            output_data[index] = client->plain[client->plain_offset + index];
        }
        client->plain_offset += plain_length;
        if (client->plain_offset == client->plain_length) {
            client->plain_offset = 0;
            client->plain_length = 0;
        }
        return plain_length;
    }
    if (client->input_length == 0) {
        received = jadren_tls_read_more(client);
        if (received == 0) {
            client->state = JADREN_TLS_STATE_PEER_CLOSED;
            return 0;
        }
        if (received < 0) return 0;
    }
    for (;;) {
        buffers[0].length = client->input_length;
        buffers[0].type = JADREN_SECBUFFER_DATA;
        buffers[0].data = client->input;
        buffers[1].length = 0; buffers[1].type = JADREN_SECBUFFER_EMPTY; buffers[1].data = 0;
        buffers[2].length = 0; buffers[2].type = JADREN_SECBUFFER_EMPTY; buffers[2].data = 0;
        buffers[3].length = 0; buffers[3].type = JADREN_SECBUFFER_EMPTY; buffers[3].data = 0;
        message.version = JADREN_SECBUFFER_VERSION;
        message.count = 4;
        message.buffers = buffers;
        status = DecryptMessage(&client->context, &message, 0, &quality);
        if (status == JADREN_SEC_E_INCOMPLETE_MESSAGE) {
            received = jadren_tls_read_more(client);
            if (received == 0) {
                client->state = JADREN_TLS_STATE_PEER_CLOSED;
                return 0;
            }
            if (received < 0) return 0;
            continue;
        }
        if (status == JADREN_SEC_I_CONTEXT_EXPIRED) {
            client->state = JADREN_TLS_STATE_PEER_CLOSED;
            client->last_error = status;
            return 0;
        }
        if (status != JADREN_SEC_E_OK) {
            client->state = JADREN_TLS_STATE_ERROR;
            client->last_error = status;
            return 0;
        }
        {
            unsigned long data_length = 0;
            unsigned long extra_length = 0;
            unsigned char *data = 0;
            unsigned char *extra = 0;
            for (index = 0; index < 4; index += 1) {
                if (buffers[index].type == JADREN_SECBUFFER_DATA) {
                    data = (unsigned char *)buffers[index].data;
                    data_length = buffers[index].length;
                } else if (buffers[index].type == JADREN_SECBUFFER_EXTRA) {
                    extra = (unsigned char *)buffers[index].data;
                    extra_length = buffers[index].length;
                }
            }
            if (data == 0 || data_length > sizeof(client->plain)) {
                client->last_error = -4;
                client->state = JADREN_TLS_STATE_ERROR;
                return 0;
            }
            for (index = 0; index < data_length; index += 1) client->plain[index] = data[index];
            client->plain_length = data_length;
            client->plain_offset = 0;
            if (extra != 0 && extra_length <= client->input_length) {
                unsigned long source = client->input_length - extra_length;
                for (index = 0; index < extra_length; index += 1) client->input[index] = extra[index];
                client->input_length = extra_length;
            } else {
                client->input_length = 0;
            }
            return net_tls_receive(token, output_data, output_length);
        }
    }
}

int net_tls_close(unsigned __int64 token) {
    JadrenTlsClient *client = jadren_tls_find(token);
    JadrenTlsServer *server = jadren_tls_find_server(token);
    int success;
    if (server != 0) {
        if (!server->active) return 0;
        if (server->context_ready) (void)DeleteSecurityContext(&server->context);
        if (server->credential_ready) (void)FreeCredentialsHandle(&server->credential);
        if (server->certificate_store != 0) (void)CertCloseStore(server->certificate_store, 0);
        success = net_socket_close(server->socket_token);
        jadren_tls_zero(server, sizeof(*server));
        return success;
    }
    if (client == 0 || !client->active) return 0;
    if (client->context_ready) (void)DeleteSecurityContext(&client->context);
    if (client->credential_ready) (void)FreeCredentialsHandle(&client->credential);
    success = net_socket_close(client->socket_token);
    jadren_tls_zero(client, sizeof(*client));
    return success;
}
#endif

int file_exists(const char *path_data, unsigned __int64 path_length) {
    wchar_t wide_path[1024];
    return path_to_wide(path_data, path_length, wide_path, 1024) &&
           GetFileAttributesW(wide_path) != INVALID_FILE_ATTRIBUTES;
}

/* Flushes an existing file through the OS. This is an explicit durability
 * boundary; directory metadata, rename transactions and database semantics
 * remain caller responsibilities. */
int file_flush(const char *path_data, unsigned __int64 path_length) {
    wchar_t wide_path[1024];
    HANDLE handle;
    int flushed;
    int closed;
    if (!path_to_wide(path_data, path_length, wide_path, 1024)) return 0;
    handle = CreateFileW(wide_path, GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                         0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
    if (handle == INVALID_HANDLE_VALUE) return 0;
    flushed = FlushFileBuffers(handle);
    closed = CloseHandle(handle);
    return flushed && closed;
}

/* Flushes an existing file through an explicit valid caller-owned UTF-8 path. */
int file_flush_path(const unsigned char *path_data, unsigned __int64 path_capacity,
                    unsigned __int64 path_length) {
    if (path_length > path_capacity) return 0;
    return file_flush((const char *)path_data, path_length);
}

/* Flushes directory metadata through the OS. This is the durability boundary
 * required after rename/delete; filesystems may still decline the request. */
int directory_flush(const char *path_data, unsigned __int64 path_length) {
    wchar_t wide_path[1024];
    HANDLE handle;
    int flushed;
    int closed;
    if (!path_to_wide(path_data, path_length, wide_path, 1024)) return 0;
    handle = CreateFileW(wide_path, GENERIC_READ | GENERIC_WRITE,
                         FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                         0, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, 0);
    if (handle == INVALID_HANDLE_VALUE) return 0;
    flushed = FlushFileBuffers(handle);
    closed = CloseHandle(handle);
    return flushed && closed;
}

int directory_exists(const char *path_data, unsigned __int64 path_length) {
    wchar_t wide_path[1024];
    unsigned long attributes;
    if (!path_to_wide(path_data, path_length, wide_path, 1024)) return 0;
    attributes = GetFileAttributesW(wide_path);
    return attributes != INVALID_FILE_ATTRIBUTES &&
           (attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
}

int directory_create(const char *path_data, unsigned __int64 path_length) {
    wchar_t wide_path[1024];
    if (!path_to_wide(path_data, path_length, wide_path, 1024)) return 0;
    if (CreateDirectoryW(wide_path, 0)) return 1;
    return directory_exists(path_data, path_length);
}

int directory_delete(const char *path_data, unsigned __int64 path_length) {
    wchar_t wide_path[1024];
    return path_to_wide(path_data, path_length, wide_path, 1024) &&
           RemoveDirectoryW(wide_path);
}

typedef struct JadrenDirectoryEntry {
    unsigned __int64 length;
    char name[1024];
} JadrenDirectoryEntry;

static int jadren_directory_entry_less(const JadrenDirectoryEntry *left,
                                       const JadrenDirectoryEntry *right) {
    unsigned __int64 index;
    unsigned __int64 limit = left->length < right->length
        ? left->length
        : right->length;
    for (index = 0; index < limit; index += 1) {
        unsigned char left_byte = (unsigned char)left->name[index];
        unsigned char right_byte = (unsigned char)right->name[index];
        if (left_byte != right_byte) return left_byte < right_byte;
    }
    return left->length < right->length;
}

static void jadren_directory_entry_swap(JadrenDirectoryEntry *left,
                                         JadrenDirectoryEntry *right) {
    unsigned __int64 byte;
    unsigned __int64 length = left->length;
    left->length = right->length;
    right->length = length;
    for (byte = 0; byte < sizeof(left->name); byte += 1) {
        char value = left->name[byte];
        left->name[byte] = right->name[byte];
        right->name[byte] = value;
    }
}

static int jadren_directory_wide_name_to_utf8(const wchar_t *source,
                                               char *target,
                                               unsigned __int64 capacity,
                                               unsigned __int64 *length) {
    int source_length = 0;
    int converted;
    if (source == 0 || target == 0 || length == 0 || capacity < 2) return 0;
    while (source_length < 260 && source[source_length] != 0) {
        source_length += 1;
    }
    if (source_length == 0 || source_length > 0x7FFFFFFFLL ||
        capacity > 0x7FFFFFFFLL) {
        return 0;
    }
    converted = WideCharToMultiByte(CP_UTF8, 0, source, source_length,
                                    target, (int)capacity, 0, 0);
    if (converted <= 0 || (unsigned __int64)converted >= capacity) return 0;
    target[converted] = 0;
    *length = (unsigned __int64)converted;
    return 1;
}

unsigned __int64 directory_list(const char *path_data,
                                unsigned __int64 path_length,
                                unsigned char *output_data,
                                unsigned __int64 output_capacity) {
    wchar_t pattern[1024];
    JadrenFindDataW find_data;
    JadrenDirectoryEntry entries[128];
    HANDLE search_handle;
    unsigned int pattern_length = 0;
    unsigned int entry_count = 0;
    unsigned int index;
    unsigned __int64 total = 0;
    if (!path_to_wide(path_data, path_length, pattern, 1024)) return 0;
    while (pattern_length < 1024 && pattern[pattern_length] != 0) {
        pattern_length += 1;
    }
    if (pattern_length == 0 || pattern_length + 2 >= 1024) return 0;
    if (pattern[pattern_length - 1] != 0x5CU &&
        pattern[pattern_length - 1] != 0x2FU) {
        pattern[pattern_length++] = 0x5CU;
    }
    pattern[pattern_length++] = 0x2AU;
    pattern[pattern_length] = 0;
    search_handle = FindFirstFileW(pattern, &find_data);
    if (search_handle == INVALID_HANDLE_VALUE) return 0;
    for (;;) {
        int is_dot = find_data.file_name[0] == 0x2EU &&
                     find_data.file_name[1] == 0;
        int is_dot_dot = find_data.file_name[0] == 0x2EU &&
                         find_data.file_name[1] == 0x2EU &&
                         find_data.file_name[2] == 0;
        if (!is_dot && !is_dot_dot) {
            if (entry_count >= 128 ||
                !jadren_directory_wide_name_to_utf8(
                    find_data.file_name, entries[entry_count].name,
                    sizeof(entries[entry_count].name),
                    &entries[entry_count].length)) {
                (void)FindClose(search_handle);
                return 0;
            }
            entry_count += 1;
        }
        if (!FindNextFileW(search_handle, &find_data)) break;
    }
    (void)FindClose(search_handle);
    for (index = 0; index < entry_count; index += 1) {
        unsigned int next;
        for (next = index + 1; next < entry_count; next += 1) {
            if (jadren_directory_entry_less(&entries[next], &entries[index])) {
                jadren_directory_entry_swap(&entries[index], &entries[next]);
            }
        }
        if (total > 0xFFFFFFFFFFFFFFFFULL - entries[index].length) return 0;
        total += entries[index].length;
        if (index + 1 < entry_count) {
            if (total == 0xFFFFFFFFFFFFFFFFULL) return 0;
            total += 1;
        }
    }
    if (total == 0 || output_data == 0 || output_capacity < total) return 0;
    total = 0;
    for (index = 0; index < entry_count; index += 1) {
        unsigned __int64 byte;
        for (byte = 0; byte < entries[index].length; byte += 1) {
            output_data[total++] = (unsigned char)entries[index].name[byte];
        }
        if (index + 1 < entry_count) output_data[total++] = 0x0AU;
    }
    return total;
}

typedef struct JadrenDirectoryEntryEx {
    unsigned __int64 length;
    unsigned char kind;
    char name[1024];
} JadrenDirectoryEntryEx;

static int jadren_directory_entry_ex_less(const JadrenDirectoryEntryEx *left,
                                          const JadrenDirectoryEntryEx *right) {
    unsigned __int64 index;
    unsigned __int64 limit = left->length < right->length
        ? left->length
        : right->length;
    for (index = 0; index < limit; index += 1) {
        unsigned char left_byte = (unsigned char)left->name[index];
        unsigned char right_byte = (unsigned char)right->name[index];
        if (left_byte != right_byte) return left_byte < right_byte;
    }
    return left->length < right->length;
}

static void jadren_directory_entry_ex_swap(JadrenDirectoryEntryEx *left,
                                            JadrenDirectoryEntryEx *right) {
    unsigned __int64 byte;
    unsigned __int64 length = left->length;
    unsigned char kind = left->kind;
    left->length = right->length;
    right->length = length;
    left->kind = right->kind;
    right->kind = kind;
    for (byte = 0; byte < sizeof(left->name); byte += 1) {
        char value = left->name[byte];
        left->name[byte] = right->name[byte];
        right->name[byte] = value;
    }
}

unsigned __int64 directory_list_ex(const char *path_data,
                                   unsigned __int64 path_length,
                                   unsigned char *names_data,
                                   unsigned __int64 names_capacity,
                                   unsigned char *kinds_data,
                                   unsigned __int64 kinds_capacity) {
    wchar_t pattern[1024];
    JadrenFindDataW find_data;
    JadrenDirectoryEntryEx entries[128];
    HANDLE search_handle;
    unsigned int pattern_length = 0;
    unsigned int entry_count = 0;
    unsigned int index;
    unsigned __int64 total = 0;
    if (!path_to_wide(path_data, path_length, pattern, 1024)) return 0;
    while (pattern_length < 1024 && pattern[pattern_length] != 0) {
        pattern_length += 1;
    }
    if (pattern_length == 0 || pattern_length + 2 >= 1024) return 0;
    if (pattern[pattern_length - 1] != 0x5CU &&
        pattern[pattern_length - 1] != 0x2FU) {
        pattern[pattern_length++] = 0x5CU;
    }
    pattern[pattern_length++] = 0x2AU;
    pattern[pattern_length] = 0;
    search_handle = FindFirstFileW(pattern, &find_data);
    if (search_handle == INVALID_HANDLE_VALUE) return 0;
    for (;;) {
        int is_dot = find_data.file_name[0] == 0x2EU &&
                     find_data.file_name[1] == 0;
        int is_dot_dot = find_data.file_name[0] == 0x2EU &&
                         find_data.file_name[1] == 0x2EU &&
                         find_data.file_name[2] == 0;
        if (!is_dot && !is_dot_dot) {
            if (entry_count >= 128 ||
                !jadren_directory_wide_name_to_utf8(
                    find_data.file_name, entries[entry_count].name,
                    sizeof(entries[entry_count].name),
                    &entries[entry_count].length)) {
                (void)FindClose(search_handle);
                return 0;
            }
            entries[entry_count].kind =
                (find_data.file_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0 ? 2 : 1;
            entry_count += 1;
        }
        if (!FindNextFileW(search_handle, &find_data)) break;
    }
    (void)FindClose(search_handle);
    for (index = 0; index < entry_count; index += 1) {
        unsigned int next;
        for (next = index + 1; next < entry_count; next += 1) {
            if (jadren_directory_entry_ex_less(&entries[next], &entries[index])) {
                jadren_directory_entry_ex_swap(&entries[index], &entries[next]);
            }
        }
        if (total > 0xFFFFFFFFFFFFFFFFULL - entries[index].length) return 0;
        total += entries[index].length;
        if (index + 1 < entry_count) {
            if (total == 0xFFFFFFFFFFFFFFFFULL) return 0;
            total += 1;
        }
    }
    if (entry_count == 0 || names_data == 0 || kinds_data == 0 ||
        names_capacity < total || kinds_capacity < entry_count) return 0;
    total = 0;
    for (index = 0; index < entry_count; index += 1) {
        unsigned __int64 byte;
        for (byte = 0; byte < entries[index].length; byte += 1) {
            names_data[total++] = (unsigned char)entries[index].name[byte];
        }
        kinds_data[index] = entries[index].kind;
        if (index + 1 < entry_count) names_data[total++] = 0x0AU;
    }
    return entry_count;
}

int directory_list_ex_exact(const char *path_data,
                            unsigned __int64 path_length,
                            unsigned char *names_data,
                            unsigned __int64 names_capacity,
                            unsigned __int64 *names_length_data,
                            unsigned __int64 names_length_capacity,
                            unsigned char *kinds_data,
                            unsigned __int64 kinds_capacity,
                            unsigned __int64 *item_count_data,
                            unsigned __int64 item_count_capacity) {
    wchar_t pattern[1024];
    JadrenFindDataW find_data;
    JadrenDirectoryEntryEx entries[128];
    HANDLE search_handle;
    unsigned int pattern_length = 0;
    unsigned int entry_count = 0;
    unsigned int index;
    unsigned __int64 total = 0;
    if (!path_to_wide(path_data, path_length, pattern, 1024) ||
        names_length_data == 0 || names_length_capacity < 1 ||
        item_count_data == 0 || item_count_capacity < 1) return 0;
    while (pattern_length < 1024 && pattern[pattern_length] != 0) {
        pattern_length += 1;
    }
    if (pattern_length == 0 || pattern_length + 2 >= 1024) return 0;
    if (pattern[pattern_length - 1] != 0x5CU &&
        pattern[pattern_length - 1] != 0x2FU) {
        pattern[pattern_length++] = 0x5CU;
    }
    pattern[pattern_length++] = 0x2AU;
    pattern[pattern_length] = 0;
    search_handle = FindFirstFileW(pattern, &find_data);
    if (search_handle == INVALID_HANDLE_VALUE) return 0;
    for (;;) {
        int is_dot = find_data.file_name[0] == 0x2EU &&
                     find_data.file_name[1] == 0;
        int is_dot_dot = find_data.file_name[0] == 0x2EU &&
                         find_data.file_name[1] == 0x2EU &&
                         find_data.file_name[2] == 0;
        if (!is_dot && !is_dot_dot) {
            if (entry_count >= 128 ||
                !jadren_directory_wide_name_to_utf8(
                    find_data.file_name, entries[entry_count].name,
                    sizeof(entries[entry_count].name),
                    &entries[entry_count].length)) {
                (void)FindClose(search_handle);
                return 0;
            }
            entries[entry_count].kind =
                (find_data.file_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0 ? 2 : 1;
            entry_count += 1;
        }
        if (!FindNextFileW(search_handle, &find_data)) break;
    }
    (void)FindClose(search_handle);
    for (index = 0; index < entry_count; index += 1) {
        unsigned int next;
        for (next = index + 1; next < entry_count; next += 1) {
            if (jadren_directory_entry_ex_less(&entries[next], &entries[index])) {
                jadren_directory_entry_ex_swap(&entries[index], &entries[next]);
            }
        }
        if (total > 0xFFFFFFFFFFFFFFFFULL - entries[index].length) return 0;
        total += entries[index].length;
        if (index + 1 < entry_count) {
            if (total == 0xFFFFFFFFFFFFFFFFULL) return 0;
            total += 1;
        }
    }
    if (names_capacity < total || kinds_capacity < entry_count ||
        (total != 0 && names_data == 0) ||
        (entry_count != 0 && kinds_data == 0)) return 0;
    total = 0;
    for (index = 0; index < entry_count; index += 1) {
        unsigned __int64 byte;
        for (byte = 0; byte < entries[index].length; byte += 1) {
            names_data[total++] = (unsigned char)entries[index].name[byte];
        }
        kinds_data[index] = entries[index].kind;
        if (index + 1 < entry_count) names_data[total++] = 0x0AU;
    }
    *names_length_data = total;
    *item_count_data = entry_count;
    return 1;
}

int file_copy(const char *source_data, unsigned __int64 source_length,
              const char *target_data, unsigned __int64 target_length) {
    wchar_t source_path[1024];
    wchar_t target_path[1024];
    if (!path_to_wide(source_data, source_length, source_path, 1024) ||
        !path_to_wide(target_data, target_length, target_path, 1024)) {
        return 0;
    }
    return CopyFileW(source_path, target_path, 1);
}

unsigned __int64 time_now_unix_seconds(void) {
    JadrenFileTime file_time;
    unsigned __int64 ticks;
    const unsigned __int64 unix_epoch_ticks = 116444736000000000ULL;
    GetSystemTimeAsFileTime(&file_time);
    ticks = ((unsigned __int64)file_time.high << 32) |
            (unsigned __int64)file_time.low;
    if (ticks < unix_epoch_ticks) {
        return 0;
    }
    return (ticks - unix_epoch_ticks) / 10000000ULL;
}

unsigned __int64 time_now_monotonic_ms(void) {
    JadrenLargeInteger counter = {0};
    JadrenLargeInteger frequency = {0};
    unsigned __int64 ticks;
    unsigned __int64 rate;
    if (!QueryPerformanceCounter(&counter) ||
        !QueryPerformanceFrequency(&frequency) ||
        counter.QuadPart < 0 || frequency.QuadPart <= 0) {
        return 0;
    }
    ticks = (unsigned __int64)counter.QuadPart;
    rate = (unsigned __int64)frequency.QuadPart;
    return (ticks / rate) * 1000ULL + ((ticks % rate) * 1000ULL) / rate;
}

int time_utc_parts(long long timestamp, int *output_data,
                   unsigned __int64 output_length) {
    long long days;
    long long remainder;
    long long z;
    long long era;
    unsigned long long doe;
    unsigned long long yoe;
    long long year;
    unsigned long long doy;
    unsigned long long month_index;
    unsigned long long day;
    unsigned long long month;
    if (output_data == 0 || output_length < 6) {
        return 0;
    }
    days = timestamp / 86400LL;
    remainder = timestamp % 86400LL;
    if (remainder < 0) {
        remainder += 86400LL;
        days -= 1;
    }
    z = days + 719468LL;
    era = z >= 0 ? z / 146097LL : (z - 146096LL) / 146097LL;
    doe = (unsigned long long)(z - era * 146097LL);
    yoe = (doe - doe / 1460ULL + doe / 36524ULL - doe / 146096ULL) / 365ULL;
    year = (long long)yoe + era * 400LL;
    doy = doe - (365ULL * yoe + yoe / 4ULL - yoe / 100ULL);
    month_index = (5ULL * doy + 2ULL) / 153ULL;
    day = doy - (153ULL * month_index + 2ULL) / 5ULL + 1ULL;
    month = month_index < 10ULL ? month_index + 3ULL : month_index - 9ULL;
    year += month <= 2ULL ? 1LL : 0LL;
    if (year < -2147483648LL || year > 2147483647LL) {
        return 0;
    }
    output_data[0] = (int)year;
    output_data[1] = (int)month;
    output_data[2] = (int)day;
    output_data[3] = (int)(remainder / 3600LL);
    output_data[4] = (int)((remainder % 3600LL) / 60LL);
    output_data[5] = (int)(remainder % 60LL);
    return 1;
}

int time_utc_offset_parts(long long timestamp, int offset_minutes,
                          int *output_data, unsigned __int64 output_length) {
    long long delta = (long long)offset_minutes * 60LL;
    const long long minimum = (-9223372036854775807LL - 1LL);
    const long long maximum = 9223372036854775807LL;
    if ((delta > 0 && timestamp > maximum - delta) ||
        (delta < 0 && timestamp < minimum - delta)) {
        return 0;
    }
    return time_utc_parts(timestamp + delta, output_data, output_length);
}

#define JADREN_SCHEDULER_CAPACITY 64
typedef struct JadrenSchedulerEntry {
    int active;
    int task_id;
    long long due_unix_seconds;
    unsigned __int64 repeat_seconds;
} JadrenSchedulerEntry;

static JadrenSchedulerEntry jadren_scheduler_entries[JADREN_SCHEDULER_CAPACITY];

static int jadren_scheduler_find(int task_id) {
    int index;
    for (index = 0; index < JADREN_SCHEDULER_CAPACITY; index += 1) {
        if (jadren_scheduler_entries[index].active &&
            jadren_scheduler_entries[index].task_id == task_id) {
            return index;
        }
    }
    return -1;
}

static int jadren_scheduler_free_slot(void) {
    int index;
    for (index = 0; index < JADREN_SCHEDULER_CAPACITY; index += 1) {
        if (!jadren_scheduler_entries[index].active) {
            return index;
        }
    }
    return -1;
}

void app_scheduler_clear(void) {
    int index;
    for (index = 0; index < JADREN_SCHEDULER_CAPACITY; index += 1) {
        jadren_scheduler_entries[index].active = 0;
    }
}

int app_scheduler_set(int task_id, long long due_unix_seconds,
                      unsigned __int64 repeat_seconds) {
    int index = jadren_scheduler_find(task_id);
    if (repeat_seconds > 9223372036854775807ULL) {
        return 0;
    }
    if (index < 0) {
        index = jadren_scheduler_free_slot();
        if (index < 0) {
            return 0;
        }
    }
    jadren_scheduler_entries[index].active = 1;
    jadren_scheduler_entries[index].task_id = task_id;
    jadren_scheduler_entries[index].due_unix_seconds = due_unix_seconds;
    jadren_scheduler_entries[index].repeat_seconds = repeat_seconds;
    return 1;
}

int app_scheduler_cancel(int task_id) {
    int index = jadren_scheduler_find(task_id);
    if (index < 0) {
        return 0;
    }
    jadren_scheduler_entries[index].active = 0;
    return 1;
}

unsigned __int64 app_scheduler_count(void) {
    unsigned __int64 count = 0;
    int index;
    for (index = 0; index < JADREN_SCHEDULER_CAPACITY; index += 1) {
        if (jadren_scheduler_entries[index].active) {
            count += 1ULL;
        }
    }
    return count;
}

unsigned __int64 app_scheduler_poll(long long now_unix_seconds,
                                    int *output_data,
                                    unsigned __int64 output_length) {
    int selected[JADREN_SCHEDULER_CAPACITY];
    int due_count = 0;
    int index;
    int position;
    if (output_data == 0 && output_length > 0) {
        return 0;
    }
    for (index = 0; index < JADREN_SCHEDULER_CAPACITY; index += 1) {
        if (jadren_scheduler_entries[index].active &&
            jadren_scheduler_entries[index].due_unix_seconds <= now_unix_seconds) {
            if ((unsigned __int64)due_count >= output_length) {
                return 0;
            }
            position = due_count;
            while (position > 0) {
                int previous = selected[position - 1];
                JadrenSchedulerEntry *left = &jadren_scheduler_entries[previous];
                JadrenSchedulerEntry *right = &jadren_scheduler_entries[index];
                if (left->due_unix_seconds < right->due_unix_seconds ||
                    (left->due_unix_seconds == right->due_unix_seconds &&
                     left->task_id <= right->task_id)) {
                    break;
                }
                selected[position] = previous;
                position -= 1;
            }
            selected[position] = index;
            due_count += 1;
        }
    }
    for (position = 0; position < due_count; position += 1) {
        JadrenSchedulerEntry *entry = &jadren_scheduler_entries[selected[position]];
        output_data[position] = entry->task_id;
    }
    for (position = 0; position < due_count; position += 1) {
        JadrenSchedulerEntry *entry = &jadren_scheduler_entries[selected[position]];
        if (entry->repeat_seconds == 0 ||
            now_unix_seconds > 9223372036854775807LL -
                (long long)entry->repeat_seconds) {
            entry->active = 0;
        } else {
            entry->due_unix_seconds =
                now_unix_seconds + (long long)entry->repeat_seconds;
        }
    }
    return (unsigned __int64)due_count;
}

int app_scheduler_poll_exact(long long now_unix_seconds,
                             int *output_data,
                             unsigned __int64 output_length,
                             unsigned __int64 *due_count_output) {
    unsigned __int64 due_count = 0;
    int index;
    if (due_count_output == 0 || (output_data == 0 && output_length > 0)) {
        return 0;
    }
    for (index = 0; index < JADREN_SCHEDULER_CAPACITY; index += 1) {
        if (jadren_scheduler_entries[index].active &&
            jadren_scheduler_entries[index].due_unix_seconds <= now_unix_seconds) {
            due_count += 1ULL;
        }
    }
    if (due_count > output_length) {
        return 0;
    }
    if (app_scheduler_poll(now_unix_seconds, output_data, output_length) != due_count) {
        return 0;
    }
    *due_count_output = due_count;
    return 1;
}

int app_scheduler_next_due_exact(long long *next_due_output,
                                 unsigned __int64 next_due_length,
                                 unsigned char *has_due_output,
                                 unsigned __int64 has_due_length) {
    int index;
    int selected = -1;
    if (next_due_output == 0 || next_due_length == 0 ||
        has_due_output == 0 || has_due_length == 0) return 0;
    for (index = 0; index < JADREN_SCHEDULER_CAPACITY; index += 1) {
        JadrenSchedulerEntry *entry = &jadren_scheduler_entries[index];
        if (!entry->active) continue;
        if (selected < 0 ||
            entry->due_unix_seconds < jadren_scheduler_entries[selected].due_unix_seconds ||
            (entry->due_unix_seconds == jadren_scheduler_entries[selected].due_unix_seconds &&
             entry->task_id < jadren_scheduler_entries[selected].task_id)) {
            selected = index;
        }
    }
    if (selected < 0) {
        *has_due_output = 0;
        return 1;
    }
    *next_due_output = jadren_scheduler_entries[selected].due_unix_seconds;
    *has_due_output = 1;
    return 1;
}

/* Deterministic caller-owned scheduler snapshot. The format is JDS1, a
 * little-endian u32 active-count, then count records of
 * {task_id:i32,due:i64,repeat:u64}; records are sorted by due then task ID. */
static void app_scheduler_snapshot_write_u32(unsigned char *output,
                                             unsigned __int64 offset,
                                             unsigned int value) {
    output[offset + 0] = (unsigned char)(value & 0xFFU);
    output[offset + 1] = (unsigned char)((value >> 8) & 0xFFU);
    output[offset + 2] = (unsigned char)((value >> 16) & 0xFFU);
    output[offset + 3] = (unsigned char)((value >> 24) & 0xFFU);
}

static void app_scheduler_snapshot_write_u64(unsigned char *output,
                                             unsigned __int64 offset,
                                             unsigned __int64 value) {
    unsigned int index;
    for (index = 0; index < 8; index += 1) {
        output[offset + index] = (unsigned char)((value >> (index * 8)) & 0xFFULL);
    }
}

static unsigned int app_scheduler_snapshot_read_u32(const unsigned char *input,
                                                    unsigned __int64 offset) {
    return (unsigned int)input[offset + 0] |
           ((unsigned int)input[offset + 1] << 8) |
           ((unsigned int)input[offset + 2] << 16) |
           ((unsigned int)input[offset + 3] << 24);
}

static unsigned __int64 app_scheduler_snapshot_read_u64(const unsigned char *input,
                                                        unsigned __int64 offset) {
    unsigned __int64 value = 0;
    unsigned int index;
    for (index = 0; index < 8; index += 1) {
        value |= ((unsigned __int64)input[offset + index]) << (index * 8);
    }
    return value;
}

int app_scheduler_write_exact(unsigned char *output_data,
                              unsigned __int64 output_length,
                              unsigned __int64 *snapshot_length_output,
                              unsigned __int64 snapshot_length_capacity) {
    int selected[JADREN_SCHEDULER_CAPACITY];
    int count = 0;
    int index;
    int position;
    unsigned __int64 required;
    if (snapshot_length_output == 0 || snapshot_length_capacity == 0) return 0;
    for (index = 0; index < JADREN_SCHEDULER_CAPACITY; index += 1) {
        JadrenSchedulerEntry *entry = &jadren_scheduler_entries[index];
        if (!entry->active) continue;
        position = count;
        while (position > 0) {
            JadrenSchedulerEntry *left = &jadren_scheduler_entries[selected[position - 1]];
            if (left->due_unix_seconds < entry->due_unix_seconds ||
                (left->due_unix_seconds == entry->due_unix_seconds &&
                 left->task_id <= entry->task_id)) break;
            selected[position] = selected[position - 1];
            position -= 1;
        }
        selected[position] = index;
        count += 1;
    }
    required = 8ULL + (unsigned __int64)count * 20ULL;
    if ((output_data == 0 && required > 0) || output_length < required) return 0;
    output_data[0] = (unsigned char)'J';
    output_data[1] = (unsigned char)'D';
    output_data[2] = (unsigned char)'S';
    output_data[3] = (unsigned char)'1';
    app_scheduler_snapshot_write_u32(output_data, 4, (unsigned int)count);
    for (index = 0; index < count; index += 1) {
        JadrenSchedulerEntry *entry = &jadren_scheduler_entries[selected[index]];
        unsigned __int64 offset = 8ULL + (unsigned __int64)index * 20ULL;
        app_scheduler_snapshot_write_u32(output_data, offset,
                                         (unsigned int)entry->task_id);
        app_scheduler_snapshot_write_u64(output_data, offset + 4,
                                         (unsigned __int64)entry->due_unix_seconds);
        app_scheduler_snapshot_write_u64(output_data, offset + 12,
                                         entry->repeat_seconds);
    }
    snapshot_length_output[0] = required;
    return 1;
}

int app_scheduler_load_exact(const unsigned char *input_data,
                             unsigned __int64 input_capacity,
                             unsigned __int64 input_length) {
    JadrenSchedulerEntry parsed[JADREN_SCHEDULER_CAPACITY];
    unsigned int count;
    unsigned int index;
    unsigned int other;
    unsigned __int64 required;
    if ((input_data == 0 && input_length > 0) || input_length > input_capacity ||
        input_length < 8ULL || input_data[0] != (unsigned char)'J' ||
        input_data[1] != (unsigned char)'D' || input_data[2] != (unsigned char)'S' ||
        input_data[3] != (unsigned char)'1') return 0;
    count = app_scheduler_snapshot_read_u32(input_data, 4);
    if (count > JADREN_SCHEDULER_CAPACITY) return 0;
    required = 8ULL + (unsigned __int64)count * 20ULL;
    if (input_length != required) return 0;
    for (index = 0; index < JADREN_SCHEDULER_CAPACITY; index += 1) {
        parsed[index].active = 0;
        parsed[index].task_id = 0;
        parsed[index].due_unix_seconds = 0;
        parsed[index].repeat_seconds = 0;
    }
    for (index = 0; index < count; index += 1) {
        unsigned __int64 offset = 8ULL + (unsigned __int64)index * 20ULL;
        unsigned int encoded_task = app_scheduler_snapshot_read_u32(input_data, offset);
        unsigned __int64 encoded_due = app_scheduler_snapshot_read_u64(input_data, offset + 4);
        unsigned __int64 repeat = app_scheduler_snapshot_read_u64(input_data, offset + 12);
        parsed[index].active = 1;
        parsed[index].task_id = (int)encoded_task;
        parsed[index].due_unix_seconds = (long long)encoded_due;
        parsed[index].repeat_seconds = repeat;
        if (repeat > 9223372036854775807ULL) return 0;
        for (other = 0; other < index; other += 1) {
            if (parsed[other].task_id == parsed[index].task_id) return 0;
        }
    }
    for (index = 0; index < JADREN_SCHEDULER_CAPACITY; index += 1) {
        jadren_scheduler_entries[index] = parsed[index];
    }
    return 1;
}

unsigned __int64 string_length(const unsigned char *data,
                               unsigned __int64 length) {
    return data == 0 && length > 0 ? 0 : length;
}

int string_equals(const unsigned char *left_data, unsigned __int64 left_length,
                  const unsigned char *right_data, unsigned __int64 right_length) {
    unsigned __int64 index;
    if ((left_data == 0 && left_length > 0) ||
        (right_data == 0 && right_length > 0) ||
        left_length != right_length) {
        return 0;
    }
    for (index = 0; index < left_length; index += 1) {
        if (left_data[index] != right_data[index]) {
            return 0;
        }
    }
    return 1;
}

unsigned __int64 string_builder_append(const unsigned char *data,
                                       unsigned __int64 length,
                                       unsigned char *output_data,
                                       unsigned __int64 output_length,
                                       unsigned __int64 offset) {
    unsigned __int64 index;
    if ((data == 0 && length > 0) || output_data == 0 ||
        offset > output_length || length > output_length - offset) {
        return 0;
    }
    for (index = 0; index < length; index += 1) {
        output_data[offset + index] = data[index];
    }
    return offset + length;
}

unsigned __int64 string_builder_append_bytes(const unsigned char *data,
                                             unsigned __int64 length,
                                             unsigned char *output_data,
                                             unsigned __int64 output_length,
                                             unsigned __int64 offset) {
    return string_builder_append(data, length, output_data, output_length, offset);
}

unsigned __int64 string_builder_append_bytes_prefix(const unsigned char *data,
                                                    unsigned __int64 data_length,
                                                    unsigned __int64 prefix_length,
                                                    unsigned char *output_data,
                                                    unsigned __int64 output_length,
                                                    unsigned __int64 offset) {
    if (prefix_length > data_length) return 0;
    return string_builder_append(data, prefix_length, output_data, output_length, offset);
}

typedef struct JadrenOwnedString {
    unsigned char *pointer;
    unsigned __int64 length;
    unsigned __int64 capacity;
} JadrenOwnedString;

typedef union JadrenOwnedStringPayload {
    JadrenOwnedString value;
    int error;
    unsigned char bytes[24];
} JadrenOwnedStringPayload;

typedef struct JadrenOwnedStringResult {
    unsigned int tag;
    JadrenOwnedStringPayload payload;
} JadrenOwnedStringResult;

static int string_owned_valid_utf8(const unsigned char *data,
                                   unsigned __int64 length) {
    unsigned __int64 index = 0;
    if (data == 0 && length > 0) return 0;
    while (index < length) {
        unsigned char lead = data[index];
        unsigned __int64 needed;
        unsigned int codepoint;
        if (lead < 0x80) {
            index += 1;
            continue;
        }
        if (lead >= 0xC2 && lead <= 0xDF) {
            needed = 2;
            codepoint = (unsigned int)(lead & 0x1F);
        } else if (lead >= 0xE0 && lead <= 0xEF) {
            needed = 3;
            codepoint = (unsigned int)(lead & 0x0F);
        } else if (lead >= 0xF0 && lead <= 0xF4) {
            needed = 4;
            codepoint = (unsigned int)(lead & 0x07);
        } else {
            return 0;
        }
        if (needed > length - index) return 0;
        if (needed == 3 && lead == 0xE0 && data[index + 1] < 0xA0) return 0;
        if (needed == 3 && lead == 0xED && data[index + 1] >= 0xA0) return 0;
        if (needed == 4 && lead == 0xF0 && data[index + 1] < 0x90) return 0;
        if (needed == 4 && lead == 0xF4 && data[index + 1] >= 0x90) return 0;
        while (needed > 1) {
            unsigned char continuation = data[index + needed - 1];
            if (continuation < 0x80 || continuation > 0xBF) return 0;
            codepoint = (codepoint << 6) | (unsigned int)(continuation & 0x3F);
            needed -= 1;
        }
        if (codepoint > 0x10FFFF || (codepoint >= 0xD800 && codepoint <= 0xDFFF)) {
            return 0;
        }
        index += (lead < 0xE0 ? 2 : (lead < 0xF0 ? 3 : 4));
    }
    return 1;
}

static JadrenOwnedStringResult string_owned_error(int status) {
    JadrenOwnedStringResult result;
    unsigned __int64 index;
    result.tag = 0;
    for (index = 0; index < sizeof(result.payload.bytes); index += 1) {
        result.payload.bytes[index] = 0;
    }
    result.payload.error = status;
    return result;
}

static JadrenOwnedStringResult string_owned_ok(JadrenOwnedString value) {
    JadrenOwnedStringResult result;
    result.tag = 1;
    result.payload.value = value;
    return result;
}

unsigned __int64 string_owned_create_capacity(unsigned __int64 capacity,
                                              JadrenOwnedString *output) {
    HANDLE heap;
    if (output == 0) return 0;
    output->pointer = 0;
    output->length = 0;
    output->capacity = 0;
    if (capacity == 0) return 1;
    heap = GetProcessHeap();
    if (heap == 0) return 0;
    output->pointer = (unsigned char *)HeapAlloc(heap, 0, (SIZE_T)capacity);
    if (output->pointer == 0) return 0;
    output->capacity = capacity;
    return 1;
}

JadrenOwnedStringResult string_owned_create(unsigned __int64 capacity) {
    JadrenOwnedString value;
    if (!string_owned_create_capacity(capacity, &value)) {
        return string_owned_error(-14);
    }
    return string_owned_ok(value);
}

JadrenOwnedStringResult string_owned_from(const unsigned char *data,
                                          unsigned __int64 length) {
    JadrenOwnedString value;
    unsigned __int64 index;
    if ((data == 0 && length > 0) || !string_owned_valid_utf8(data, length)) {
        return string_owned_error(data == 0 && length > 0 ? -15 : -40);
    }
    if (!string_owned_create_capacity(length, &value)) {
        return string_owned_error(-14);
    }
    for (index = 0; index < length; index += 1) value.pointer[index] = data[index];
    value.length = length;
    return string_owned_ok(value);
}

int string_owned_reserve(JadrenOwnedString *value, unsigned __int64 minimum_capacity) {
    HANDLE heap;
    unsigned __int64 grown;
    void *resized;
    if (value == 0) return -15;
    if (value->pointer == 0 && value->capacity != 0) return -41;
    if (value->length > value->capacity) return -41;
    if (minimum_capacity <= value->capacity) return 0;
    grown = value->capacity == 0 ? 16 : value->capacity;
    while (grown < minimum_capacity) {
        if (grown > 0x7FFFFFFFFFFFFFFFULL) {
            grown = minimum_capacity;
            break;
        }
        grown *= 2;
    }
    heap = GetProcessHeap();
    if (heap == 0) return -10;
    resized = value->pointer == 0
        ? HeapAlloc(heap, 0, (SIZE_T)grown)
        : HeapReAlloc(heap, 0, value->pointer, (SIZE_T)grown);
    if (resized == 0) return -14;
    value->pointer = (unsigned char *)resized;
    value->capacity = grown;
    return 0;
}

int string_owned_append(JadrenOwnedString *value,
                        const unsigned char *data,
                        unsigned __int64 length) {
    unsigned __int64 new_length;
    unsigned __int64 index;
    int status;
    if (value == 0) return -15;
    if ((data == 0 && length > 0) || !string_owned_valid_utf8(data, length)) {
        return data == 0 && length > 0 ? -15 : -40;
    }
    if (value->length > value->capacity) return -41;
    if (value->length > 0 && !string_owned_valid_utf8(value->pointer, value->length)) {
        return -41;
    }
    if (length > 0xFFFFFFFFFFFFFFFFULL - value->length) return -13;
    new_length = value->length + length;
    status = string_owned_reserve(value, new_length);
    if (status != 0) return status;
    for (index = 0; index < length; index += 1) value->pointer[value->length + index] = data[index];
    value->length = new_length;
    return 0;
}

unsigned __int64 string_owned_length(const unsigned char *data,
                                     unsigned __int64 length) {
    return string_owned_valid_utf8(data, length) ? length : 0;
}

unsigned __int64 string_owned_copy(const unsigned char *data,
                                   unsigned __int64 length,
                                   unsigned char *output_data,
                                   unsigned __int64 output_length) {
    unsigned __int64 index;
    if (!string_owned_valid_utf8(data, length) || output_data == 0 ||
        length > output_length) return 0;
    for (index = 0; index < length; index += 1) output_data[index] = data[index];
    return length;
}

int string_owned_clear(JadrenOwnedString *value) {
    if (value == 0) return -15;
    if (value->pointer == 0 && value->capacity != 0) return -41;
    if (value->length > value->capacity) return -41;
    value->length = 0;
    return 0;
}

int string_owned_destroy(JadrenOwnedString *value) {
    HANDLE heap;
    if (value == 0) return -15;
    if (value->pointer == 0 && value->capacity != 0) return -41;
    if (value->length > value->capacity) return -41;
    if (value->pointer != 0) {
        heap = GetProcessHeap();
        if (heap == 0 || !HeapFree(heap, 0, value->pointer)) return -10;
    }
    value->pointer = 0;
    value->length = 0;
    value->capacity = 0;
    return 0;
}

int jadren_rt_owned_string_destroy(JadrenOwnedString *value) {
    return string_owned_destroy(value);
}

unsigned __int64 file_size(const char *path_data, unsigned __int64 path_length) {
    wchar_t wide_path[1024];
    JadrenLargeInteger size = {0};
    HANDLE handle;
    if (!path_to_wide(path_data, path_length, wide_path, 1024)) {
        return 0;
    }
    handle = CreateFileW(wide_path, GENERIC_READ,
                         FILE_SHARE_READ | FILE_SHARE_WRITE, 0,
                         OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
    if (handle == INVALID_HANDLE_VALUE) {
        return 0;
    }
    if (!GetFileSizeEx(handle, &size) || size.QuadPart < 0) {
        CloseHandle(handle);
        return 0;
    }
    CloseHandle(handle);
    return (unsigned __int64)size.QuadPart;
}

unsigned __int64 file_read(const char *path_data, unsigned __int64 path_length,
                           unsigned char *output_data, unsigned __int64 output_length) {
    wchar_t wide_path[1024];
    HANDLE handle;
    unsigned __int64 total = 0;
    if (!path_to_wide(path_data, path_length, wide_path, 1024) ||
        output_data == 0 || output_length == 0) {
        return 0;
    }
    handle = CreateFileW(wide_path, GENERIC_READ,
                         FILE_SHARE_READ | FILE_SHARE_WRITE, 0,
                         OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
    if (handle == INVALID_HANDLE_VALUE) {
        return 0;
    }
    while (total < output_length) {
        unsigned __int64 remaining = output_length - total;
        DWORD requested = remaining > 0xFFFFFFFFULL ? 0xFFFFFFFFUL : (DWORD)remaining;
        DWORD received = 0;
        if (!ReadFile(handle, output_data + total, requested, &received, 0) || received == 0) {
            break;
        }
        total += (unsigned __int64)received;
        if (received < requested) {
            break;
        }
    }
    CloseHandle(handle);
    return total;
}

unsigned __int64 file_read_at(const char *path_data, unsigned __int64 path_length,
                              unsigned __int64 offset,
                              unsigned char *output_data, unsigned __int64 output_length) {
    wchar_t wide_path[1024];
    HANDLE handle;
    JadrenLargeInteger distance = {0};
    JadrenLargeInteger position = {0};
    unsigned __int64 total = 0;
    if (!path_to_wide(path_data, path_length, wide_path, 1024) ||
        output_data == 0 || output_length == 0 ||
        offset > 0x7FFFFFFFFFFFFFFFULL) {
        return 0;
    }
    handle = CreateFileW(wide_path, GENERIC_READ,
                         FILE_SHARE_READ | FILE_SHARE_WRITE, 0,
                         OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
    if (handle == INVALID_HANDLE_VALUE) {
        return 0;
    }
    distance.QuadPart = (LONGLONG)offset;
    if (!SetFilePointerEx(handle, distance, &position, FILE_BEGIN)) {
        CloseHandle(handle);
        return 0;
    }
    while (total < output_length) {
        unsigned __int64 remaining = output_length - total;
        DWORD requested = remaining > 0xFFFFFFFFULL ? 0xFFFFFFFFUL : (DWORD)remaining;
        DWORD received = 0;
        if (!ReadFile(handle, output_data + total, requested, &received, 0) || received == 0) {
            break;
        }
        total += (unsigned __int64)received;
        if (received < requested) {
            break;
        }
    }
    CloseHandle(handle);
    return total;
}

unsigned __int64 file_size_path(const unsigned char *path_data,
                                unsigned __int64 path_capacity,
                                unsigned __int64 path_length) {
    if (path_data == 0 || path_length > path_capacity) return 0;
    return file_size((const char *)path_data, path_length);
}

int file_exists_path(const unsigned char *path_data,
                     unsigned __int64 path_capacity,
                     unsigned __int64 path_length) {
    if (path_data == 0 || path_length > path_capacity) return 0;
    return file_exists((const char *)path_data, path_length);
}

int file_path_valid(const unsigned char *path_data,
                    unsigned __int64 path_capacity,
                    unsigned __int64 path_length) {
    wchar_t wide_path[1024];
    if (path_data == 0 || path_length > path_capacity) return 0;
    return path_to_wide((const char *)path_data, path_length,
                        wide_path, 1024);
}

unsigned __int64 file_mtime_unix_nanos_path(const unsigned char *path_data,
                                            unsigned __int64 path_capacity,
                                            unsigned __int64 path_length) {
    wchar_t wide_path[1024];
    JadrenFindDataW metadata;
    HANDLE search_handle;
    const unsigned __int64 windows_epoch_100ns = 116444736000000000ULL;
    unsigned __int64 timestamp_100ns;
    unsigned __int64 elapsed_100ns;
    if (path_data == 0 || path_length > path_capacity ||
        !path_to_wide((const char *)path_data, path_length, wide_path, 1024)) {
        return 0;
    }
    search_handle = FindFirstFileW(wide_path, &metadata);
    if (search_handle == INVALID_HANDLE_VALUE) return 0;
    (void)FindClose(search_handle);
    timestamp_100ns = ((unsigned __int64)metadata.last_write_time_high << 32) |
                       (unsigned __int64)metadata.last_write_time_low;
    if (timestamp_100ns <= windows_epoch_100ns) return 0;
    elapsed_100ns = timestamp_100ns - windows_epoch_100ns;
    if (elapsed_100ns > 184467440737095516ULL) return 0;
    return elapsed_100ns * 100ULL;
}

unsigned __int64 file_read_at_path(const unsigned char *path_data,
                                   unsigned __int64 path_capacity,
                                   unsigned __int64 path_length,
                                   unsigned __int64 offset,
                                   unsigned char *output_data,
                                   unsigned __int64 output_length) {
    if (path_data == 0 || path_length > path_capacity) return 0;
    return file_read_at((const char *)path_data, path_length, offset,
                        output_data, output_length);
}

unsigned __int64 file_read_text(const char *path_data, unsigned __int64 path_length,
                                unsigned char *output_data,
                                unsigned __int64 output_length) {
    unsigned __int64 size;
    unsigned __int64 loaded;
    if (path_data == 0 || output_data == 0 || output_length == 0 ||
        !file_exists(path_data, path_length)) {
        return 0;
    }
    size = file_size(path_data, path_length);
    if (size >= output_length) return 0;
    loaded = file_read(path_data, path_length, output_data, size);
    if (loaded != size) return 0;
    output_data[size] = 0;
    return size;
}

int file_read_exact(const char *path_data, unsigned __int64 path_length,
                    unsigned char *output_data, unsigned __int64 output_length,
                    unsigned __int64 *output_size,
                    unsigned __int64 output_size_capacity) {
    unsigned __int64 size;
    unsigned __int64 loaded;
    if (path_data == 0 || output_size == 0 || output_size_capacity == 0 ||
        !file_exists(path_data, path_length)) {
        return 0;
    }
    size = file_size(path_data, path_length);
    if (output_length < size || (output_data == 0 && size > 0)) {
        return 0;
    }
    loaded = size == 0 ? 0 : file_read(path_data, path_length, output_data, size);
    if (loaded != size) {
        return 0;
    }
    output_size[0] = size;
    return 1;
}

int file_read_text_exact(const char *path_data, unsigned __int64 path_length,
                         unsigned char *output_data, unsigned __int64 output_length,
                         unsigned __int64 *output_size,
                         unsigned __int64 output_size_capacity) {
    unsigned __int64 size;
    unsigned __int64 loaded;
    if (path_data == 0 || output_data == 0 || output_size == 0 ||
        output_size_capacity == 0 || !file_exists(path_data, path_length)) {
        return 0;
    }
    size = file_size(path_data, path_length);
    if (size >= output_length) {
        return 0;
    }
    loaded = size == 0 ? 0 : file_read(path_data, path_length, output_data, size);
    if (loaded != size) {
        return 0;
    }
    output_data[size] = 0;
    output_size[0] = size;
    return 1;
}

static unsigned __int64 write_file_bytes(const char *path_data,
                                         unsigned __int64 path_length,
                                         const unsigned char *input_data,
                                         unsigned __int64 input_length) {
    wchar_t wide_path[1024];
    HANDLE handle;
    unsigned __int64 total = 0;
    if (!path_to_wide(path_data, path_length, wide_path, 1024) ||
        (input_data == 0 && input_length > 0)) {
        return 0;
    }
    handle = CreateFileW(wide_path, GENERIC_WRITE, FILE_SHARE_READ, 0,
                         CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    if (handle == INVALID_HANDLE_VALUE) {
        return 0;
    }
    while (total < input_length) {
        unsigned __int64 remaining = input_length - total;
        DWORD requested = remaining > 0xFFFFFFFFULL ? 0xFFFFFFFFUL : (DWORD)remaining;
        DWORD written = 0;
        if (!WriteFile(handle, input_data + total, requested, &written, 0) || written == 0) {
            break;
        }
        total += (unsigned __int64)written;
        if (written < requested) {
            break;
        }
    }
    CloseHandle(handle);
    return total;
}

unsigned __int64 file_write(const char *path_data, unsigned __int64 path_length,
                            const unsigned char *input_data,
                            unsigned __int64 input_length) {
    return write_file_bytes(path_data, path_length, input_data, input_length);
}

/* Truncating write from an explicit valid prefix of a larger caller-owned buffer. */
unsigned __int64 file_write_prefix(const char *path_data, unsigned __int64 path_length,
                                   const unsigned char *input_data,
                                   unsigned __int64 input_length,
                                   unsigned __int64 write_length) {
    if (write_length > input_length) return 0;
    return write_file_bytes(path_data, path_length, input_data, write_length);
}

/* Truncating write through an explicit valid prefix of a caller-owned UTF-8 path. */
unsigned __int64 file_write_prefix_path(const unsigned char *path_data,
                                        unsigned __int64 path_capacity,
                                        unsigned __int64 path_length,
                                        const unsigned char *input_data,
                                        unsigned __int64 input_capacity,
                                        unsigned __int64 write_length) {
    if (path_length > path_capacity || write_length > input_capacity) return 0;
    return write_file_bytes((const char *)path_data, path_length, input_data, write_length);
}

unsigned __int64 file_write_at(const char *path_data, unsigned __int64 path_length,
                               unsigned __int64 offset,
                               const unsigned char *input_data,
                               unsigned __int64 input_length) {
    wchar_t wide_path[1024];
    HANDLE handle;
    JadrenLargeInteger distance = {0};
    JadrenLargeInteger position = {0};
    unsigned __int64 total = 0;
    if (!path_to_wide(path_data, path_length, wide_path, 1024) ||
        (input_data == 0 && input_length > 0) ||
        offset > 0x7FFFFFFFFFFFFFFFULL) {
        return 0;
    }
    handle = CreateFileW(wide_path, GENERIC_WRITE, FILE_SHARE_READ, 0,
                         OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    if (handle == INVALID_HANDLE_VALUE) {
        return 0;
    }
    distance.QuadPart = (LONGLONG)offset;
    if (!SetFilePointerEx(handle, distance, &position, FILE_BEGIN)) {
        CloseHandle(handle);
        return 0;
    }
    while (total < input_length) {
        unsigned __int64 remaining = input_length - total;
        DWORD requested = remaining > 0xFFFFFFFFULL ? 0xFFFFFFFFUL : (DWORD)remaining;
        DWORD written = 0;
        if (!WriteFile(handle, input_data + total, requested, &written, 0) || written == 0) {
            break;
        }
        total += (unsigned __int64)written;
        if (written < requested) {
            break;
        }
    }
    CloseHandle(handle);
    return total;
}

unsigned __int64 file_write_text(const char *path_data, unsigned __int64 path_length,
                                 const char *content_data,
                                 unsigned __int64 content_length) {
    return write_file_bytes(path_data, path_length,
                            (const unsigned char *)content_data, content_length);
}

static unsigned __int64 append_file_bytes(const char *path_data,
                                          unsigned __int64 path_length,
                                          const unsigned char *content_data,
                                          unsigned __int64 content_length) {
    wchar_t wide_path[1024];
    HANDLE handle;
    unsigned __int64 total = 0;
    if (!path_to_wide(path_data, path_length, wide_path, 1024) ||
        (content_data == 0 && content_length > 0)) {
        return 0;
    }
    handle = CreateFileW(wide_path, FILE_APPEND_DATA, FILE_SHARE_READ, 0,
                         OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    if (handle == INVALID_HANDLE_VALUE) {
        return 0;
    }
    while (total < content_length) {
        unsigned __int64 remaining = content_length - total;
        DWORD requested = remaining > 0xFFFFFFFFULL ? 0xFFFFFFFFUL : (DWORD)remaining;
        DWORD written = 0;
        if (!WriteFile(handle, content_data + total, requested, &written, 0) || written == 0) {
            break;
        }
        total += (unsigned __int64)written;
        if (written < requested) {
            break;
        }
    }
    CloseHandle(handle);
    return total;
}

unsigned __int64 file_append_text(const char *path_data, unsigned __int64 path_length,
                                  const char *content_data,
                                  unsigned __int64 content_length) {
    return append_file_bytes(path_data, path_length,
                             (const unsigned char *)content_data, content_length);
}

unsigned __int64 file_append(const char *path_data, unsigned __int64 path_length,
                             const unsigned char *input_data,
                             unsigned __int64 input_length,
                             unsigned __int64 append_length) {
    if (append_length > input_length) {
        return 0;
    }
    return append_file_bytes(path_data, path_length, input_data, append_length);
}

unsigned __int64 format_uint(unsigned __int64 value, unsigned char *output_data,
                             unsigned __int64 output_length) {
    unsigned char reversed[20];
    unsigned __int64 digits = 0;
    unsigned __int64 index;
    if (output_data == 0 || output_length == 0) {
        return 0;
    }
    do {
        reversed[digits] = (unsigned char)('0' + (value % 10));
        digits += 1;
        value /= 10;
    } while (value != 0);
    if (digits > output_length) {
        return 0;
    }
    for (index = 0; index < digits; index += 1) {
        output_data[index] = reversed[digits - index - 1];
    }
    return digits;
}

unsigned __int64 format_hex_uint(unsigned __int64 value, unsigned char *output_data,
                                 unsigned __int64 output_length) {
    unsigned char reversed[16];
    unsigned __int64 digits = 0;
    unsigned __int64 index;
    if (output_data == 0 || output_length == 0) {
        return 0;
    }
    do {
        unsigned char digit = (unsigned char)(value & 0xFULL);
        reversed[digits] = (unsigned char)(digit < 10 ? ('0' + digit) : ('a' + digit - 10));
        digits += 1;
        value >>= 4;
    } while (value != 0);
    if (digits > output_length) {
        return 0;
    }
    for (index = 0; index < digits; index += 1) {
        output_data[index] = reversed[digits - index - 1];
    }
    return digits;
}

unsigned __int64 format_int(long long value, unsigned char *output_data,
                            unsigned __int64 output_length) {
    unsigned char reversed[20];
    unsigned long long magnitude;
    unsigned long long digits = 0;
    unsigned long long required;
    unsigned long long index;
    int negative = value < 0;
    if (output_data == 0 || output_length == 0) {
        return 0;
    }
    magnitude = negative ? (0ULL - (unsigned long long)value) : (unsigned long long)value;
    do {
        reversed[digits] = (unsigned char)('0' + (magnitude % 10));
        digits += 1;
        magnitude /= 10;
    } while (magnitude != 0);
    required = digits + (negative ? 1ULL : 0ULL);
    if (required > output_length) {
        return 0;
    }
    if (negative) {
        output_data[0] = (unsigned char)'-';
    }
    for (index = 0; index < digits; index += 1) {
        output_data[(negative ? 1ULL : 0ULL) + index] = reversed[digits - index - 1];
    }
    return required;
}

unsigned __int64 format_bool(unsigned char value, unsigned char *output_data,
                             unsigned __int64 output_length) {
    const char *text = value ? "true" : "false";
    unsigned __int64 length = value ? 4ULL : 5ULL;
    unsigned __int64 index;
    if (output_data == 0 || output_length < length) {
        return 0;
    }
    for (index = 0; index < length; index += 1) {
        output_data[index] = (unsigned char)text[index];
    }
    return length;
}

static unsigned __int64 copy_format_literal(const char *text,
                                            unsigned __int64 length,
                                            unsigned char *output_data,
                                            unsigned __int64 output_length) {
    unsigned __int64 index;
    if (output_data == 0 || output_length < length) {
        return 0;
    }
    for (index = 0; index < length; index += 1) {
        output_data[index] = (unsigned char)text[index];
    }
    return length;
}

unsigned __int64 format_float(double value, unsigned char *output_data,
                              unsigned __int64 output_length) {
    union {
        double value;
        unsigned long long bits;
    } representation;
    unsigned long long bits;
    unsigned long long fraction;
    unsigned long long significand;
    unsigned long long scaled;
    unsigned long long integer_part;
    unsigned long long fractional_part;
    unsigned char integer_text[20];
    unsigned __int64 integer_length;
    unsigned __int64 required;
    unsigned __int64 index;
    unsigned __int64 offset = 0;
    unsigned __int64 divisor;
    unsigned int exponent;
    int shift;
    int negative;
    unsigned __int128 scaled_wide;
    representation.value = value;
    bits = representation.bits;
    negative = (bits >> 63) != 0;
    exponent = (unsigned int)((bits >> 52) & 0x7FFU);
    fraction = bits & 0x000FFFFFFFFFFFFFULL;
    if (exponent == 0x7FFU) {
        if (fraction != 0) {
            return copy_format_literal("nan", 3, output_data, output_length);
        }
        if (negative) {
            return copy_format_literal("-inf", 4, output_data, output_length);
        }
        return copy_format_literal("inf", 3, output_data, output_length);
    }
    if (exponent == 0) {
        significand = fraction;
        shift = -1074;
    } else {
        significand = (1ULL << 52) | fraction;
        shift = (int)exponent - 1023 - 52;
    }
    if (shift >= 0) {
        if (shift > 55) {
            return 0;
        }
        scaled_wide = ((unsigned __int128)significand << shift) * 1000000U;
    } else {
        unsigned int divisor_shift = (unsigned int)(-shift);
        scaled_wide = (unsigned __int128)significand * 1000000U;
        if (divisor_shift > 127) {
            scaled_wide = 0;
        } else if (divisor_shift > 0) {
            scaled_wide += (unsigned __int128)1 << (divisor_shift - 1);
            scaled_wide >>= divisor_shift;
        }
    }
    if ((scaled_wide >> 64) != 0) {
        return 0;
    }
    scaled = (unsigned long long)scaled_wide;
    integer_part = scaled / 1000000ULL;
    fractional_part = scaled % 1000000ULL;
    integer_length = format_uint(integer_part, integer_text, sizeof(integer_text));
    required = integer_length + 7 + (negative ? 1ULL : 0ULL);
    if (integer_length == 0 || output_data == 0 || output_length < required) {
        return 0;
    }
    if (negative) {
        output_data[offset] = (unsigned char)'-';
        offset += 1;
    }
    for (index = 0; index < integer_length; index += 1) {
        output_data[offset + index] = integer_text[index];
    }
    offset += integer_length;
    output_data[offset] = (unsigned char)'.';
    offset += 1;
    divisor = 100000ULL;
    for (index = 0; index < 6; index += 1) {
        output_data[offset + index] = (unsigned char)('0' + ((fractional_part / divisor) % 10ULL));
        divisor /= 10ULL;
    }
    return required;
}

static const char *http_reason_text(unsigned short status,
                                    unsigned __int64 *length) {
    switch (status) {
    case 200: *length = 2; return "OK";
    case 201: *length = 7; return "Created";
    case 204: *length = 10; return "No Content";
    case 301: *length = 5; return "Moved";
    case 302: *length = 5; return "Found";
    case 400: *length = 11; return "Bad Request";
    case 401: *length = 12; return "Unauthorized";
    case 403: *length = 9; return "Forbidden";
    case 404: *length = 9; return "Not Found";
    case 405: *length = 18; return "Method Not Allowed";
    case 409: *length = 8; return "Conflict";
    case 422: *length = 20; return "Unprocessable Entity";
    case 429: *length = 17; return "Too Many Requests";
    case 500: *length = 21; return "Internal Server Error";
    case 501: *length = 14; return "Not Implemented";
    case 503: *length = 19; return "Service Unavailable";
    default: *length = 7; return "Unknown";
    }
}

static int http_add_length(unsigned __int64 *total,
                           unsigned __int64 addition) {
    if (total == 0 || *total > ((unsigned __int64)-1) - addition) {
        return 0;
    }
    *total += addition;
    return 1;
}

static int http_copy_bytes(unsigned char *output_data,
                           unsigned __int64 *offset,
                           const unsigned char *input_data,
                           unsigned __int64 input_length) {
    unsigned __int64 index;
    if (output_data == 0 || offset == 0 ||
        (input_data == 0 && input_length > 0)) {
        return 0;
    }
    for (index = 0; index < input_length; index += 1) {
        output_data[*offset + index] = input_data[index];
    }
    *offset += input_length;
    return 1;
}

static unsigned __int64 http_response_write_mode(unsigned short status,
                                                 const char *content_type_data,
                                                 unsigned __int64 content_type_length,
                                                 const unsigned char *body_data,
                                                 unsigned __int64 body_length,
                                                 int keep_alive,
                                                 unsigned char *output_data,
                                                 unsigned __int64 output_length) {
    const char *reason;
    unsigned __int64 reason_length;
    unsigned char status_text[20];
    unsigned char body_length_text[20];
    unsigned __int64 status_length;
    unsigned __int64 body_text_length;
    unsigned __int64 required = 0;
    unsigned __int64 offset = 0;
    unsigned __int64 index;
    static const unsigned char prefix[] = "HTTP/1.1 ";
    static const unsigned char type_prefix[] = "\r\nContent-Type: ";
    static const unsigned char length_prefix[] = "\r\nContent-Length: ";
    static const unsigned char close_suffix[] = "\r\nConnection: close\r\n\r\n";
    static const unsigned char keep_suffix[] = "\r\nConnection: keep-alive\r\n\r\n";
    const unsigned char *suffix = keep_alive ? keep_suffix : close_suffix;
    unsigned __int64 suffix_length = keep_alive ? sizeof(keep_suffix) - 1 : sizeof(close_suffix) - 1;
    if (status < 100 || status > 999 || content_type_data == 0 ||
        content_type_length == 0 || (body_data == 0 && body_length > 0)) {
        return 0;
    }
    for (index = 0; index < content_type_length; index += 1) {
        unsigned char value = (unsigned char)content_type_data[index];
        if (value < 0x20 || value > 0x7E) {
            return 0;
        }
    }
    reason = http_reason_text(status, &reason_length);
    status_length = format_uint(status, status_text, sizeof(status_text));
    body_text_length = format_uint(body_length, body_length_text,
                                   sizeof(body_length_text));
    if (status_length == 0 || body_text_length == 0 ||
        !http_add_length(&required, sizeof(prefix) - 1) ||
        !http_add_length(&required, status_length) ||
        !http_add_length(&required, 1) ||
        !http_add_length(&required, reason_length) ||
        !http_add_length(&required, sizeof(type_prefix) - 1) ||
        !http_add_length(&required, content_type_length) ||
        !http_add_length(&required, sizeof(length_prefix) - 1) ||
        !http_add_length(&required, body_text_length) ||
        !http_add_length(&required, suffix_length) ||
        !http_add_length(&required, body_length) ||
        output_data == 0 || output_length < required) {
        return 0;
    }
    http_copy_bytes(output_data, &offset, prefix, sizeof(prefix) - 1);
    http_copy_bytes(output_data, &offset, status_text, status_length);
    http_copy_bytes(output_data, &offset, (const unsigned char *)" ", 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)reason, reason_length);
    http_copy_bytes(output_data, &offset, type_prefix, sizeof(type_prefix) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)content_type_data,
                    content_type_length);
    http_copy_bytes(output_data, &offset, length_prefix, sizeof(length_prefix) - 1);
    http_copy_bytes(output_data, &offset, body_length_text, body_text_length);
    http_copy_bytes(output_data, &offset, suffix, suffix_length);
    http_copy_bytes(output_data, &offset, body_data, body_length);
    return required;
}

unsigned __int64 http_response_write(unsigned short status,
                                     const char *content_type_data,
                                     unsigned __int64 content_type_length,
                                     const unsigned char *body_data,
                                     unsigned __int64 body_length,
                                     unsigned char *output_data,
                                     unsigned __int64 output_length) {
    return http_response_write_mode(status, content_type_data, content_type_length,
                                    body_data, body_length, 0, output_data, output_length);
}

unsigned __int64 http_response_write_ex(unsigned short status,
                                        const char *content_type_data,
                                        unsigned __int64 content_type_length,
                                        const unsigned char *body_data,
                                        unsigned __int64 body_length,
                                        int keep_alive,
                                        unsigned char *output_data,
                                        unsigned __int64 output_length) {
    return http_response_write_mode(status, content_type_data, content_type_length,
                                    body_data, body_length, keep_alive, output_data, output_length);
}

unsigned __int64 http_response_write_prefix_ex(unsigned short status,
                                               const char *content_type_data,
                                               unsigned __int64 content_type_length,
                                               const unsigned char *body_data,
                                               unsigned __int64 body_capacity,
                                               unsigned __int64 body_length,
                                               int keep_alive,
                                               unsigned char *output_data,
                                               unsigned __int64 output_length) {
    if (body_length > body_capacity) return 0;
    return http_response_write_mode(status, content_type_data, content_type_length,
                                    body_data, body_length, keep_alive, output_data, output_length);
}

static unsigned __int64 http_response_write_chunked_prefix_mode(
    unsigned short status,
    const char *content_type_data,
    unsigned __int64 content_type_length,
    const unsigned char *body_data,
    unsigned __int64 body_capacity,
    unsigned __int64 body_length,
    unsigned char *output_data,
    unsigned __int64 output_length) {
    const char *reason;
    unsigned __int64 reason_length;
    unsigned char status_text[20];
    unsigned char chunk_length_text[16];
    unsigned __int64 status_length;
    unsigned __int64 chunk_text_length;
    unsigned __int64 required = 0;
    unsigned __int64 offset = 0;
    unsigned __int64 index;
    static const unsigned char prefix[] = "HTTP/1.1 ";
    static const unsigned char type_prefix[] = "\r\nContent-Type: ";
    static const unsigned char transfer_prefix[] = "\r\nTransfer-Encoding: chunked";
    static const unsigned char suffix[] = "\r\nConnection: close\r\n\r\n";
    static const unsigned char chunk_separator[] = "\r\n";
    static const unsigned char final_chunk[] = "\r\n0\r\n\r\n";
    static const unsigned char empty_final_chunk[] = "0\r\n\r\n";
    if (status < 100 || status > 999 || content_type_data == 0 ||
        content_type_length == 0 || body_length > body_capacity ||
        (body_data == 0 && body_length > 0)) {
        return 0;
    }
    for (index = 0; index < content_type_length; index += 1) {
        unsigned char value = (unsigned char)content_type_data[index];
        if (value < 0x20 || value > 0x7E) {
            return 0;
        }
    }
    reason = http_reason_text(status, &reason_length);
    status_length = format_uint(status, status_text, sizeof(status_text));
    chunk_text_length = format_hex_uint(body_length, chunk_length_text,
                                        sizeof(chunk_length_text));
    if (status_length == 0 || chunk_text_length == 0 ||
        !http_add_length(&required, sizeof(prefix) - 1) ||
        !http_add_length(&required, status_length) ||
        !http_add_length(&required, 1) ||
        !http_add_length(&required, reason_length) ||
        !http_add_length(&required, sizeof(type_prefix) - 1) ||
        !http_add_length(&required, content_type_length) ||
        !http_add_length(&required, sizeof(transfer_prefix) - 1) ||
        !http_add_length(&required, sizeof(suffix) - 1)) {
        return 0;
    }
    if (body_length == 0) {
        if (!http_add_length(&required, sizeof(empty_final_chunk) - 1)) {
            return 0;
        }
    } else if (!http_add_length(&required, chunk_text_length) ||
               !http_add_length(&required, sizeof(chunk_separator) - 1) ||
               !http_add_length(&required, body_length) ||
               !http_add_length(&required, sizeof(final_chunk) - 1)) {
        return 0;
    }
    if (output_data == 0 || output_length < required) {
        return 0;
    }
    http_copy_bytes(output_data, &offset, prefix, sizeof(prefix) - 1);
    http_copy_bytes(output_data, &offset, status_text, status_length);
    http_copy_bytes(output_data, &offset, (const unsigned char *)" ", 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)reason, reason_length);
    http_copy_bytes(output_data, &offset, type_prefix, sizeof(type_prefix) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)content_type_data,
                    content_type_length);
    http_copy_bytes(output_data, &offset, transfer_prefix, sizeof(transfer_prefix) - 1);
    http_copy_bytes(output_data, &offset, suffix, sizeof(suffix) - 1);
    if (body_length == 0) {
        http_copy_bytes(output_data, &offset, empty_final_chunk,
                        sizeof(empty_final_chunk) - 1);
    } else {
        http_copy_bytes(output_data, &offset, chunk_length_text, chunk_text_length);
        http_copy_bytes(output_data, &offset, chunk_separator,
                        sizeof(chunk_separator) - 1);
        http_copy_bytes(output_data, &offset, body_data, body_length);
        http_copy_bytes(output_data, &offset, final_chunk, sizeof(final_chunk) - 1);
    }
    return required;
}

unsigned __int64 http_response_write_chunked(unsigned short status,
                                             const char *content_type_data,
                                             unsigned __int64 content_type_length,
                                             const unsigned char *body_data,
                                             unsigned __int64 body_length,
                                             unsigned char *output_data,
                                             unsigned __int64 output_length) {
    return http_response_write_chunked_prefix_mode(status, content_type_data,
                                                   content_type_length, body_data,
                                                   body_length, body_length,
                                                   output_data, output_length);
}

unsigned __int64 http_response_write_chunked_prefix(
    unsigned short status,
    const char *content_type_data,
    unsigned __int64 content_type_length,
    const unsigned char *body_data,
    unsigned __int64 body_capacity,
    unsigned __int64 body_length,
    unsigned char *output_data,
    unsigned __int64 output_length) {
    return http_response_write_chunked_prefix_mode(status, content_type_data,
                                                   content_type_length, body_data,
                                                   body_capacity, body_length,
                                                   output_data, output_length);
}

/* Writes only the bounded response header for a caller-driven chunk stream.
 * The caller owns connection policy and must send the returned header before
 * one or more http_response_write_chunk calls. */
unsigned __int64 http_response_write_chunked_header(
    unsigned short status,
    const char *content_type_data,
    unsigned __int64 content_type_length,
    unsigned char *output_data,
    unsigned __int64 output_length) {
    const char *reason;
    unsigned __int64 reason_length;
    unsigned char status_text[20];
    unsigned __int64 status_length;
    unsigned __int64 required = 0;
    unsigned __int64 offset = 0;
    unsigned __int64 index;
    static const unsigned char prefix[] = "HTTP/1.1 ";
    static const unsigned char type_prefix[] = "\r\nContent-Type: ";
    static const unsigned char transfer_prefix[] = "\r\nTransfer-Encoding: chunked";
    static const unsigned char suffix[] = "\r\n\r\n";
    if (status < 100 || status > 999 || content_type_data == 0 ||
        content_type_length == 0) {
        return 0;
    }
    for (index = 0; index < content_type_length; index += 1) {
        unsigned char value = (unsigned char)content_type_data[index];
        if (value < 0x20 || value > 0x7E) {
            return 0;
        }
    }
    reason = http_reason_text(status, &reason_length);
    status_length = format_uint(status, status_text, sizeof(status_text));
    if (status_length == 0 ||
        !http_add_length(&required, sizeof(prefix) - 1) ||
        !http_add_length(&required, status_length) ||
        !http_add_length(&required, 1) ||
        !http_add_length(&required, reason_length) ||
        !http_add_length(&required, sizeof(type_prefix) - 1) ||
        !http_add_length(&required, content_type_length) ||
        !http_add_length(&required, sizeof(transfer_prefix) - 1) ||
        !http_add_length(&required, sizeof(suffix) - 1) ||
        output_data == 0 || output_length < required) {
        return 0;
    }
    http_copy_bytes(output_data, &offset, prefix, sizeof(prefix) - 1);
    http_copy_bytes(output_data, &offset, status_text, status_length);
    http_copy_bytes(output_data, &offset, (const unsigned char *)" ", 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)reason, reason_length);
    http_copy_bytes(output_data, &offset, type_prefix, sizeof(type_prefix) - 1);
    http_copy_bytes(output_data, &offset,
                    (const unsigned char *)content_type_data,
                    content_type_length);
    http_copy_bytes(output_data, &offset, transfer_prefix,
                    sizeof(transfer_prefix) - 1);
    http_copy_bytes(output_data, &offset, suffix, sizeof(suffix) - 1);
    return required;
}

/* Writes one caller-driven HTTP/1.1 chunk. A non-final chunk must contain at
 * least one byte. The final call appends the mandatory zero-length chunk, so
 * callers can send each returned frame immediately without a hidden buffer or
 * runtime-owned stream state. */
unsigned __int64 http_response_write_chunk(
    const unsigned char *body_data,
    unsigned __int64 body_length,
    int final_chunk,
    unsigned char *output_data,
    unsigned __int64 output_length) {
    unsigned char chunk_length_text[16];
    unsigned __int64 chunk_text_length;
    unsigned __int64 required = 0;
    unsigned __int64 offset = 0;
    static const unsigned char separator[] = "\r\n";
    static const unsigned char final_suffix[] = "0\r\n\r\n";
    if ((body_data == 0 && body_length > 0) ||
        (final_chunk == 0 && body_length == 0)) {
        return 0;
    }
    if (final_chunk && body_length == 0) {
        if (output_data == 0 || output_length < sizeof(final_suffix) - 1) {
            return 0;
        }
        http_copy_bytes(output_data, &offset, final_suffix,
                        sizeof(final_suffix) - 1);
        return sizeof(final_suffix) - 1;
    }
    chunk_text_length = format_hex_uint(body_length, chunk_length_text,
                                        sizeof(chunk_length_text));
    if (chunk_text_length == 0 ||
        !http_add_length(&required, chunk_text_length) ||
        !http_add_length(&required, sizeof(separator) - 1) ||
        !http_add_length(&required, body_length) ||
        !http_add_length(&required, sizeof(separator) - 1) ||
        (final_chunk && !http_add_length(&required, sizeof(final_suffix) - 1)) ||
        output_data == 0 || output_length < required) {
        return 0;
    }
    http_copy_bytes(output_data, &offset, chunk_length_text, chunk_text_length);
    http_copy_bytes(output_data, &offset, separator, sizeof(separator) - 1);
    http_copy_bytes(output_data, &offset, body_data, body_length);
    http_copy_bytes(output_data, &offset, separator, sizeof(separator) - 1);
    if (final_chunk) {
        http_copy_bytes(output_data, &offset, final_suffix,
                        sizeof(final_suffix) - 1);
    }
    return required;
}

unsigned __int64 http_response_write_chunk_prefix(
    const unsigned char *body_data,
    unsigned __int64 body_capacity,
    unsigned __int64 body_length,
    int final_chunk,
    unsigned char *output_data,
    unsigned __int64 output_length) {
    if (body_length > body_capacity) {
        return 0;
    }
    return http_response_write_chunk(body_data, body_length, final_chunk,
                                     output_data, output_length);
}

static int http_response_header_name_is_token(unsigned char value) {
    return value >= 0x21 && value <= 0x7E && value != '(' && value != ')' &&
           value != '<' && value != '>' && value != '@' && value != ',' &&
           value != ';' && value != ':' && value != '\\' && value != '"' &&
           value != '/' && value != '[' && value != ']' && value != '?' &&
           value != '=' && value != '{' && value != '}';
}

static unsigned __int64 http_response_write_header_mode(
    unsigned short status,
    const char *content_type_data,
    unsigned __int64 content_type_length,
    const char *header_name_data,
    unsigned __int64 header_name_length,
    const char *header_value_data,
    unsigned __int64 header_value_length,
    const unsigned char *body_data,
    unsigned __int64 body_length,
    int keep_alive,
    unsigned char *output_data,
    unsigned __int64 output_length) {
    const char *reason;
    unsigned __int64 reason_length;
    unsigned char status_text[20];
    unsigned char body_length_text[20];
    unsigned __int64 status_length;
    unsigned __int64 body_text_length;
    unsigned __int64 required = 0;
    unsigned __int64 offset = 0;
    unsigned __int64 index;
    static const unsigned char prefix[] = "HTTP/1.1 ";
    static const unsigned char type_prefix[] = "\r\nContent-Type: ";
    static const unsigned char length_prefix[] = "\r\nContent-Length: ";
    static const unsigned char header_prefix[] = "\r\n";
    static const unsigned char header_separator[] = ": ";
    static const unsigned char close_suffix[] = "\r\nConnection: close\r\n\r\n";
    static const unsigned char keep_suffix[] = "\r\nConnection: keep-alive\r\n\r\n";
    const unsigned char *suffix = keep_alive ? keep_suffix : close_suffix;
    unsigned __int64 suffix_length = keep_alive ? sizeof(keep_suffix) - 1 : sizeof(close_suffix) - 1;
    if (status < 100 || status > 999 || content_type_data == 0 ||
        content_type_length == 0 || header_name_data == 0 || header_name_length == 0 ||
        (header_value_data == 0 && header_value_length > 0) ||
        (body_data == 0 && body_length > 0)) {
        return 0;
    }
    for (index = 0; index < content_type_length; index += 1) {
        unsigned char value = (unsigned char)content_type_data[index];
        if (value < 0x20 || value > 0x7E) return 0;
    }
    for (index = 0; index < header_name_length; index += 1) {
        if (!http_response_header_name_is_token((unsigned char)header_name_data[index])) return 0;
    }
    for (index = 0; index < header_value_length; index += 1) {
        unsigned char value = (unsigned char)header_value_data[index];
        if (value < 0x20 || value > 0x7E) return 0;
    }
    reason = http_reason_text(status, &reason_length);
    status_length = format_uint(status, status_text, sizeof(status_text));
    body_text_length = format_uint(body_length, body_length_text, sizeof(body_length_text));
    if (status_length == 0 || body_text_length == 0 ||
        !http_add_length(&required, sizeof(prefix) - 1) ||
        !http_add_length(&required, status_length) ||
        !http_add_length(&required, 1) ||
        !http_add_length(&required, reason_length) ||
        !http_add_length(&required, sizeof(type_prefix) - 1) ||
        !http_add_length(&required, content_type_length) ||
        !http_add_length(&required, sizeof(length_prefix) - 1) ||
        !http_add_length(&required, body_text_length) ||
        !http_add_length(&required, sizeof(header_prefix) - 1) ||
        !http_add_length(&required, header_name_length) ||
        !http_add_length(&required, sizeof(header_separator) - 1) ||
        !http_add_length(&required, header_value_length) ||
        !http_add_length(&required, suffix_length) ||
        !http_add_length(&required, body_length) ||
        output_data == 0 || output_length < required) {
        return 0;
    }
    http_copy_bytes(output_data, &offset, prefix, sizeof(prefix) - 1);
    http_copy_bytes(output_data, &offset, status_text, status_length);
    http_copy_bytes(output_data, &offset, (const unsigned char *)" ", 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)reason, reason_length);
    http_copy_bytes(output_data, &offset, type_prefix, sizeof(type_prefix) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)content_type_data, content_type_length);
    http_copy_bytes(output_data, &offset, length_prefix, sizeof(length_prefix) - 1);
    http_copy_bytes(output_data, &offset, body_length_text, body_text_length);
    http_copy_bytes(output_data, &offset, header_prefix, sizeof(header_prefix) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)header_name_data, header_name_length);
    http_copy_bytes(output_data, &offset, header_separator, sizeof(header_separator) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)header_value_data, header_value_length);
    http_copy_bytes(output_data, &offset, suffix, suffix_length);
    http_copy_bytes(output_data, &offset, body_data, body_length);
    return required;
}

unsigned __int64 http_response_write_header(unsigned short status,
                                            const char *content_type_data,
                                            unsigned __int64 content_type_length,
                                            const char *header_name_data,
                                            unsigned __int64 header_name_length,
                                            const char *header_value_data,
                                            unsigned __int64 header_value_length,
                                            const unsigned char *body_data,
                                            unsigned __int64 body_length,
                                            unsigned char *output_data,
                                            unsigned __int64 output_length) {
    return http_response_write_header_mode(status, content_type_data, content_type_length,
                                           header_name_data, header_name_length,
                                           header_value_data, header_value_length,
                                           body_data, body_length, 0, output_data, output_length);
}

unsigned __int64 http_response_write_header_prefix(unsigned short status,
                                                   const char *content_type_data,
                                                   unsigned __int64 content_type_length,
                                                   const char *header_name_data,
                                                   unsigned __int64 header_name_length,
                                                   const char *header_value_data,
                                                   unsigned __int64 header_value_length,
                                                   const unsigned char *body_data,
                                                   unsigned __int64 body_capacity,
                                                   unsigned __int64 body_length,
                                                   unsigned char *output_data,
                                                   unsigned __int64 output_length) {
    if (body_length > body_capacity) return 0;
    return http_response_write_header_mode(status, content_type_data, content_type_length,
                                           header_name_data, header_name_length,
                                           header_value_data, header_value_length,
                                           body_data, body_length, 0, output_data, output_length);
}

unsigned __int64 http_response_write_header_ex(unsigned short status,
                                               const char *content_type_data,
                                               unsigned __int64 content_type_length,
                                               const char *header_name_data,
                                               unsigned __int64 header_name_length,
                                               const char *header_value_data,
                                               unsigned __int64 header_value_length,
                                               const unsigned char *body_data,
                                               unsigned __int64 body_length,
                                               int keep_alive,
                                               unsigned char *output_data,
                                               unsigned __int64 output_length) {
    return http_response_write_header_mode(status, content_type_data, content_type_length,
                                           header_name_data, header_name_length,
                                           header_value_data, header_value_length,
                                           body_data, body_length, keep_alive, output_data, output_length);
}

static int http_cookie_name_is_token(unsigned char value) {
    return http_response_header_name_is_token(value);
}

static int http_cookie_value_is_octet(unsigned char value) {
    if (value < 0x21 || value > 0x7E) return 0;
    if (value == '"' || value == ',' || value == ';' || value == '\\') return 0;
    return 1;
}

static int http_cookie_attributes_are_valid(const char *data,
                                            unsigned __int64 length) {
    unsigned __int64 index;
    if (data == 0 && length > 0) return 0;
    for (index = 0; index < length; index += 1) {
        unsigned char value = (unsigned char)data[index];
        if (value < 0x20 || value > 0x7E) return 0;
    }
    return 1;
}

static unsigned __int64 http_response_write_cookie_mode(
    unsigned short status,
    const char *content_type_data,
    unsigned __int64 content_type_length,
    const char *cookie_name_data,
    unsigned __int64 cookie_name_length,
    const char *cookie_value_data,
    unsigned __int64 cookie_value_length,
    const char *cookie_attributes_data,
    unsigned __int64 cookie_attributes_length,
    const unsigned char *body_data,
    unsigned __int64 body_length,
    int keep_alive,
    unsigned char *output_data,
    unsigned __int64 output_length) {
    unsigned char cookie_data[4096];
    unsigned __int64 cookie_length = 0;
    unsigned __int64 index;
    if (cookie_name_data == 0 || cookie_name_length == 0 ||
        (cookie_value_data == 0 && cookie_value_length > 0) ||
        !http_cookie_attributes_are_valid(cookie_attributes_data, cookie_attributes_length)) {
        return 0;
    }
    for (index = 0; index < cookie_name_length; index += 1) {
        if (!http_cookie_name_is_token((unsigned char)cookie_name_data[index])) return 0;
    }
    for (index = 0; index < cookie_value_length; index += 1) {
        if (!http_cookie_value_is_octet((unsigned char)cookie_value_data[index])) return 0;
    }
    if (!http_add_length(&cookie_length, cookie_name_length) ||
        !http_add_length(&cookie_length, 1) ||
        !http_add_length(&cookie_length, cookie_value_length) ||
        (cookie_attributes_length > 0 &&
         (!http_add_length(&cookie_length, 2) ||
          !http_add_length(&cookie_length, cookie_attributes_length))) ||
        cookie_length > sizeof(cookie_data)) {
        return 0;
    }
    index = 0;
    http_copy_bytes(cookie_data, &index, (const unsigned char *)cookie_name_data, cookie_name_length);
    http_copy_bytes(cookie_data, &index, (const unsigned char *)"=", 1);
    http_copy_bytes(cookie_data, &index, (const unsigned char *)cookie_value_data, cookie_value_length);
    if (cookie_attributes_length > 0) {
        http_copy_bytes(cookie_data, &index, (const unsigned char *)"; ", 2);
        http_copy_bytes(cookie_data, &index, (const unsigned char *)cookie_attributes_data, cookie_attributes_length);
    }
    return http_response_write_header_mode(status, content_type_data, content_type_length,
                                           "Set-Cookie", 10, (const char *)cookie_data,
                                           cookie_length, body_data, body_length, keep_alive,
                                           output_data, output_length);
}

unsigned __int64 http_response_write_cookie(unsigned short status,
                                            const char *content_type_data,
                                            unsigned __int64 content_type_length,
                                            const char *cookie_name_data,
                                            unsigned __int64 cookie_name_length,
                                            const char *cookie_value_data,
                                            unsigned __int64 cookie_value_length,
                                            const char *cookie_attributes_data,
                                            unsigned __int64 cookie_attributes_length,
                                            const unsigned char *body_data,
                                            unsigned __int64 body_length,
                                            unsigned char *output_data,
                                            unsigned __int64 output_length) {
    return http_response_write_cookie_mode(status, content_type_data, content_type_length,
                                           cookie_name_data, cookie_name_length,
                                           cookie_value_data, cookie_value_length,
                                           cookie_attributes_data, cookie_attributes_length,
                                           body_data, body_length, 0, output_data, output_length);
}

unsigned __int64 http_response_write_cookie_ex(unsigned short status,
                                               const char *content_type_data,
                                               unsigned __int64 content_type_length,
                                               const char *cookie_name_data,
                                               unsigned __int64 cookie_name_length,
                                               const char *cookie_value_data,
                                               unsigned __int64 cookie_value_length,
                                               const char *cookie_attributes_data,
                                               unsigned __int64 cookie_attributes_length,
                                               const unsigned char *body_data,
                                               unsigned __int64 body_length,
                                               int keep_alive,
                                               unsigned char *output_data,
                                               unsigned __int64 output_length) {
    return http_response_write_cookie_mode(status, content_type_data, content_type_length,
                                           cookie_name_data, cookie_name_length,
                                           cookie_value_data, cookie_value_length,
                                           cookie_attributes_data, cookie_attributes_length,
                                           body_data, body_length, keep_alive,
                                           output_data, output_length);
}

static int http_cookie_policy_value_is_valid(const char *data,
                                             unsigned __int64 length) {
    unsigned __int64 index;
    if (data == 0 && length > 0) return 0;
    for (index = 0; index < length; index += 1) {
        unsigned char value = (unsigned char)data[index];
        if (value < 0x21 || value > 0x7E || value == ';' || value == ',') return 0;
    }
    return 1;
}

static int http_cookie_policy_append(unsigned char *output_data,
                                     unsigned __int64 *output_length,
                                     unsigned __int64 output_capacity,
                                     const unsigned char *input_data,
                                     unsigned __int64 input_length) {
    unsigned __int64 previous_length;
    if (output_data == 0 || output_length == 0 ||
        (input_data == 0 && input_length > 0) ||
        *output_length > output_capacity) {
        return 0;
    }
    previous_length = *output_length;
    if (!http_add_length(output_length, input_length) || *output_length > output_capacity) return 0;
    *output_length = previous_length;
    return http_copy_bytes(output_data, output_length, input_data, input_length);
}

static unsigned __int64 http_response_write_cookie_policy_mode(
    unsigned short status,
    const char *content_type_data,
    unsigned __int64 content_type_length,
    const char *cookie_name_data,
    unsigned __int64 cookie_name_length,
    const char *cookie_value_data,
    unsigned __int64 cookie_value_length,
    const char *path_data,
    unsigned __int64 path_length,
    const char *domain_data,
    unsigned __int64 domain_length,
    long long max_age_seconds,
    unsigned int same_site,
    unsigned int flags,
    const unsigned char *body_data,
    unsigned __int64 body_length,
    int keep_alive,
    unsigned char *output_data,
    unsigned __int64 output_length) {
    unsigned char attributes[4096];
    unsigned char max_age_text[32];
    unsigned char cookie_data[4096];
    unsigned __int64 attributes_length = 0;
    unsigned __int64 max_age_length = 0;
    unsigned __int64 cookie_length = 0;
    unsigned __int64 index;
    const char *same_site_text = 0;
    unsigned __int64 same_site_length = 0;
    if (cookie_name_data == 0 || cookie_name_length == 0 ||
        (cookie_value_data == 0 && cookie_value_length > 0) ||
        max_age_seconds < -1 || same_site > 3 || (flags & ~3U) != 0 ||
        (same_site == 3 && (flags & 1U) == 0) ||
        !http_cookie_policy_value_is_valid(path_data, path_length) ||
        !http_cookie_policy_value_is_valid(domain_data, domain_length)) {
        return 0;
    }
    for (index = 0; index < cookie_name_length; index += 1) {
        if (!http_cookie_name_is_token((unsigned char)cookie_name_data[index])) return 0;
    }
    for (index = 0; index < cookie_value_length; index += 1) {
        if (!http_cookie_value_is_octet((unsigned char)cookie_value_data[index])) return 0;
    }
    if (path_length > 0) {
        if (!http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)"Path=", 5) ||
            !http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)path_data, path_length)) {
            return 0;
        }
    }
    if (domain_length > 0) {
        if (attributes_length > 0 &&
            !http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)"; ", 2)) {
            return 0;
        }
        if (!http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)"Domain=", 7) ||
            !http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)domain_data, domain_length)) {
            return 0;
        }
    }
    if (max_age_seconds != -1) {
        max_age_length = format_int(max_age_seconds, max_age_text, sizeof(max_age_text));
        if (max_age_length == 0) return 0;
        if (attributes_length > 0 &&
            !http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)"; ", 2)) {
            return 0;
        }
        if (!http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)"Max-Age=", 8) ||
            !http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       max_age_text, max_age_length)) {
            return 0;
        }
    }
    if (same_site != 0) {
        if (same_site == 1) {
            same_site_text = "Lax";
            same_site_length = 3;
        } else if (same_site == 2) {
            same_site_text = "Strict";
            same_site_length = 6;
        } else {
            same_site_text = "None";
            same_site_length = 4;
        }
        if (attributes_length > 0 &&
            !http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)"; ", 2)) {
            return 0;
        }
        if (!http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)"SameSite=", 9) ||
            !http_cookie_policy_append(attributes, &attributes_length,
                                       sizeof(attributes),
                                       (const unsigned char *)same_site_text,
                                       same_site_length)) {
            return 0;
        }
    }
    if ((flags & 1U) != 0) {
        if (attributes_length > 0 &&
            !http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)"; ", 2)) {
            return 0;
        }
        if (!http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)"Secure", 6)) {
            return 0;
        }
    }
    if ((flags & 2U) != 0) {
        if (attributes_length > 0 &&
            !http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)"; ", 2)) {
            return 0;
        }
        if (!http_cookie_policy_append(attributes, &attributes_length, sizeof(attributes),
                                       (const unsigned char *)"HttpOnly", 8)) {
            return 0;
        }
    }
    if (!http_add_length(&cookie_length, cookie_name_length) ||
        !http_add_length(&cookie_length, 1) ||
        !http_add_length(&cookie_length, cookie_value_length) ||
        (attributes_length > 0 &&
         (!http_add_length(&cookie_length, 2) ||
          !http_add_length(&cookie_length, attributes_length))) ||
        cookie_length > sizeof(cookie_data)) {
        return 0;
    }
    index = 0;
    http_copy_bytes(cookie_data, &index, (const unsigned char *)cookie_name_data, cookie_name_length);
    http_copy_bytes(cookie_data, &index, (const unsigned char *)"=", 1);
    http_copy_bytes(cookie_data, &index, (const unsigned char *)cookie_value_data, cookie_value_length);
    if (attributes_length > 0) {
        http_copy_bytes(cookie_data, &index, (const unsigned char *)"; ", 2);
        http_copy_bytes(cookie_data, &index, attributes, attributes_length);
    }
    return http_response_write_header_mode(status, content_type_data, content_type_length,
                                           "Set-Cookie", 10, (const char *)cookie_data,
                                           cookie_length, body_data, body_length, keep_alive,
                                           output_data, output_length);
}

unsigned __int64 http_response_write_cookie_policy(unsigned short status,
                                                   const char *content_type_data,
                                                   unsigned __int64 content_type_length,
                                                   const char *cookie_name_data,
                                                   unsigned __int64 cookie_name_length,
                                                   const char *cookie_value_data,
                                                   unsigned __int64 cookie_value_length,
                                                   const char *path_data,
                                                   unsigned __int64 path_length,
                                                   const char *domain_data,
                                                   unsigned __int64 domain_length,
                                                   long long max_age_seconds,
                                                   unsigned int same_site,
                                                   unsigned int flags,
                                                   const unsigned char *body_data,
                                                   unsigned __int64 body_length,
                                                   int keep_alive,
                                                   unsigned char *output_data,
                                                   unsigned __int64 output_length) {
    return http_response_write_cookie_policy_mode(status, content_type_data, content_type_length,
                                                  cookie_name_data, cookie_name_length,
                                                  cookie_value_data, cookie_value_length,
                                                  path_data, path_length, domain_data, domain_length,
                                                  max_age_seconds, same_site, flags,
                                                  body_data, body_length, keep_alive,
                                                  output_data, output_length);
}

static int http_response_header_name_is_reserved(const unsigned char *name_data,
                                                 unsigned __int64 name_length) {
    static const char *reserved[] = {"content-type", "content-length", "connection", "transfer-encoding"};
    static const unsigned __int64 lengths[] = {12, 14, 10, 17};
    unsigned __int64 item;
    unsigned __int64 index;
    if (name_data == 0 || name_length == 0) return 0;
    for (item = 0; item < sizeof(lengths) / sizeof(lengths[0]); item += 1) {
        if (name_length != lengths[item]) continue;
        for (index = 0; index < name_length; index += 1) {
            unsigned char value = (unsigned char)name_data[index];
            unsigned char expected = (unsigned char)reserved[item][index];
            if (value >= 'A' && value <= 'Z') value = (unsigned char)(value + ('a' - 'A'));
            if (value != expected) break;
        }
        if (index == name_length) return 1;
    }
    return 0;
}

static int http_response_header_block_is_valid(const unsigned char *header_data,
                                               unsigned __int64 header_length) {
    unsigned __int64 index = 0;
    while (index < header_length) {
        unsigned __int64 line_start = index;
        unsigned __int64 colon = (unsigned __int64)-1;
        unsigned __int64 name_length;
        while (index < header_length && header_data[index] != '\r' && header_data[index] != '\n') {
            unsigned char value = header_data[index];
            if (value < 0x20 || value > 0x7E) return 0;
            if (value == ':' && colon == (unsigned __int64)-1) colon = index;
            index += 1;
        }
        if (colon == (unsigned __int64)-1 || colon == line_start || index == line_start) return 0;
        name_length = colon - line_start;
        for (unsigned __int64 name_index = 0; name_index < name_length; name_index += 1) {
            if (!http_response_header_name_is_token(header_data[line_start + name_index])) return 0;
        }
        if (http_response_header_name_is_reserved(header_data + line_start, name_length)) return 0;
        if (index == header_length) return 1;
        if (header_data[index] != '\r' || index + 1 >= header_length || header_data[index + 1] != '\n') return 0;
        index += 2;
        if (index == header_length) return 0;
    }
    return 1;
}

static unsigned __int64 http_response_write_header_block_mode(
    unsigned short status,
    const char *content_type_data,
    unsigned __int64 content_type_length,
    const unsigned char *header_data,
    unsigned __int64 header_length,
    const unsigned char *body_data,
    unsigned __int64 body_length,
    int keep_alive,
    unsigned char *output_data,
    unsigned __int64 output_length) {
    const char *reason;
    unsigned __int64 reason_length;
    unsigned char status_text[20];
    unsigned char body_length_text[20];
    unsigned __int64 status_length;
    unsigned __int64 body_text_length;
    unsigned __int64 required = 0;
    unsigned __int64 offset = 0;
    unsigned __int64 index;
    static const unsigned char prefix[] = "HTTP/1.1 ";
    static const unsigned char type_prefix[] = "\r\nContent-Type: ";
    static const unsigned char length_prefix[] = "\r\nContent-Length: ";
    static const unsigned char header_prefix[] = "\r\n";
    static const unsigned char close_suffix[] = "\r\nConnection: close\r\n\r\n";
    static const unsigned char keep_suffix[] = "\r\nConnection: keep-alive\r\n\r\n";
    const unsigned char *suffix = keep_alive ? keep_suffix : close_suffix;
    unsigned __int64 suffix_length = keep_alive ? sizeof(keep_suffix) - 1 : sizeof(close_suffix) - 1;
    if (status < 100 || status > 999 || content_type_data == 0 || content_type_length == 0 ||
        (header_data == 0 && header_length > 0) || (body_data == 0 && body_length > 0) ||
        !http_response_header_block_is_valid(header_data, header_length)) {
        return 0;
    }
    for (index = 0; index < content_type_length; index += 1) {
        unsigned char value = (unsigned char)content_type_data[index];
        if (value < 0x20 || value > 0x7E) return 0;
    }
    reason = http_reason_text(status, &reason_length);
    status_length = format_uint(status, status_text, sizeof(status_text));
    body_text_length = format_uint(body_length, body_length_text, sizeof(body_length_text));
    if (status_length == 0 || body_text_length == 0 ||
        !http_add_length(&required, sizeof(prefix) - 1) ||
        !http_add_length(&required, status_length) || !http_add_length(&required, 1) ||
        !http_add_length(&required, reason_length) ||
        !http_add_length(&required, sizeof(type_prefix) - 1) ||
        !http_add_length(&required, content_type_length) ||
        !http_add_length(&required, sizeof(length_prefix) - 1) ||
        !http_add_length(&required, body_text_length) ||
        (header_length > 0 && (!http_add_length(&required, sizeof(header_prefix) - 1) ||
                               !http_add_length(&required, header_length))) ||
        !http_add_length(&required, suffix_length) || !http_add_length(&required, body_length) ||
        output_data == 0 || output_length < required) {
        return 0;
    }
    http_copy_bytes(output_data, &offset, prefix, sizeof(prefix) - 1);
    http_copy_bytes(output_data, &offset, status_text, status_length);
    http_copy_bytes(output_data, &offset, (const unsigned char *)" ", 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)reason, reason_length);
    http_copy_bytes(output_data, &offset, type_prefix, sizeof(type_prefix) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)content_type_data, content_type_length);
    http_copy_bytes(output_data, &offset, length_prefix, sizeof(length_prefix) - 1);
    http_copy_bytes(output_data, &offset, body_length_text, body_text_length);
    if (header_length > 0) {
        http_copy_bytes(output_data, &offset, header_prefix, sizeof(header_prefix) - 1);
        http_copy_bytes(output_data, &offset, header_data, header_length);
    }
    http_copy_bytes(output_data, &offset, suffix, suffix_length);
    http_copy_bytes(output_data, &offset, body_data, body_length);
    return required;
}

unsigned __int64 http_response_write_header_block(unsigned short status,
                                                  const char *content_type_data,
                                                  unsigned __int64 content_type_length,
                                                  const unsigned char *header_data,
                                                  unsigned __int64 header_length,
                                                  const unsigned char *body_data,
                                                  unsigned __int64 body_length,
                                                  unsigned char *output_data,
                                                  unsigned __int64 output_length) {
    return http_response_write_header_block_mode(status, content_type_data, content_type_length,
                                                 header_data, header_length, body_data, body_length,
                                                 0, output_data, output_length);
}

unsigned __int64 http_response_write_header_block_ex(unsigned short status,
                                                     const char *content_type_data,
                                                     unsigned __int64 content_type_length,
                                                     const unsigned char *header_data,
                                                     unsigned __int64 header_length,
                                                     const unsigned char *body_data,
                                                     unsigned __int64 body_length,
                                                     int keep_alive,
                                                     unsigned char *output_data,
                                                     unsigned __int64 output_length) {
    return http_response_write_header_block_mode(status, content_type_data, content_type_length,
                                                 header_data, header_length, body_data, body_length,
                                                 keep_alive, output_data, output_length);
}

static int http_request_is_token(unsigned char value);
static int http_request_header_name_is_reserved(const char *name_data,
                                                unsigned __int64 name_length);

unsigned __int64 http_request_write_prefix_ex(const char *method_data,
                                           unsigned __int64 method_length,
                                           const char *target_data,
                                           unsigned __int64 target_length,
                                           const char *host_data,
                                           unsigned __int64 host_length,
                                           const unsigned char *body_data,
                                           unsigned __int64 body_capacity,
                                           unsigned __int64 body_length,
                                           int keep_alive,
                                           unsigned char *output_data,
                                           unsigned __int64 output_length) {
    static const unsigned char target_prefix[] = " HTTP/1.1\r\nHost: ";
    static const unsigned char length_prefix[] = "\r\nContent-Length: ";
    static const unsigned char close_suffix[] = "\r\nConnection: close\r\n\r\n";
    static const unsigned char keep_alive_suffix[] = "\r\nConnection: keep-alive\r\n\r\n";
    const unsigned char *suffix = keep_alive == 1 ? keep_alive_suffix : close_suffix;
    unsigned __int64 suffix_length = keep_alive == 1 ? sizeof(keep_alive_suffix) - 1 : sizeof(close_suffix) - 1;
    unsigned char body_length_text[20];
    unsigned __int64 body_text_length;
    unsigned __int64 required = 0;
    unsigned __int64 offset = 0;
    unsigned __int64 index;
    if (method_data == 0 || method_length == 0 || target_data == 0 || target_length == 0 ||
        host_data == 0 || host_length == 0 || body_length > body_capacity ||
        (body_data == 0 && body_length > 0)) return 0;
    for (index = 0; index < method_length; index += 1) {
        if (!http_request_is_token((unsigned char)method_data[index])) return 0;
    }
    for (index = 0; index < target_length; index += 1) {
        unsigned char value = (unsigned char)target_data[index];
        if (value < 0x21U || value > 0x7EU || value == '\r' || value == '\n') return 0;
    }
    for (index = 0; index < host_length; index += 1) {
        unsigned char value = (unsigned char)host_data[index];
        if (value < 0x21U || value > 0x7EU || value == '\r' || value == '\n') return 0;
    }
    body_text_length = format_uint(body_length, body_length_text, sizeof(body_length_text));
    if (body_text_length == 0 || !http_add_length(&required, method_length) ||
        !http_add_length(&required, 1) || !http_add_length(&required, target_length) ||
        !http_add_length(&required, sizeof(target_prefix) - 1) ||
        !http_add_length(&required, host_length) ||
        !http_add_length(&required, sizeof(length_prefix) - 1) ||
        !http_add_length(&required, body_text_length) ||
        !http_add_length(&required, suffix_length) ||
        !http_add_length(&required, body_length) || output_data == 0 ||
        output_length < required) return 0;
    http_copy_bytes(output_data, &offset, (const unsigned char *)method_data, method_length);
    http_copy_bytes(output_data, &offset, (const unsigned char *)" ", 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)target_data, target_length);
    http_copy_bytes(output_data, &offset, target_prefix, sizeof(target_prefix) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)host_data, host_length);
    http_copy_bytes(output_data, &offset, length_prefix, sizeof(length_prefix) - 1);
    http_copy_bytes(output_data, &offset, body_length_text, body_text_length);
    http_copy_bytes(output_data, &offset, suffix, suffix_length);
    http_copy_bytes(output_data, &offset, body_data, body_length);
    return required;
}

unsigned __int64 http_request_write_prefix(const char *method_data,
                                           unsigned __int64 method_length,
                                           const char *target_data,
                                           unsigned __int64 target_length,
                                           const char *host_data,
                                           unsigned __int64 host_length,
                                           const unsigned char *body_data,
                                           unsigned __int64 body_capacity,
                                           unsigned __int64 body_length,
                                           unsigned char *output_data,
                                           unsigned __int64 output_length) {
    return http_request_write_prefix_ex(method_data, method_length, target_data,
                                        target_length, host_data, host_length,
                                        body_data, body_capacity, body_length, 0,
                                        output_data, output_length);
}

unsigned __int64 http_request_write(const char *method_data,
                                    unsigned __int64 method_length,
                                    const char *target_data,
                                    unsigned __int64 target_length,
                                    const char *host_data,
                                    unsigned __int64 host_length,
                                    const unsigned char *body_data,
                                    unsigned __int64 body_length,
                                    unsigned char *output_data,
                                    unsigned __int64 output_length) {
    return http_request_write_prefix(method_data, method_length, target_data,
                                     target_length, host_data, host_length,
                                     body_data, body_length, body_length,
                                     output_data, output_length);
}

static int http_request_header_block_is_valid(const char *header_data,
                                              unsigned __int64 header_length) {
    unsigned __int64 index = 0;
    if (header_data == 0) return header_length == 0;
    while (index < header_length) {
        unsigned __int64 line_start = index;
        unsigned __int64 colon = (unsigned __int64)-1;
        unsigned __int64 name_length;
        while (index < header_length && header_data[index] != '\r' &&
               header_data[index] != '\n') {
            unsigned char value = (unsigned char)header_data[index];
            if (value < 0x20U || value > 0x7EU) return 0;
            if (value == ':' && colon == (unsigned __int64)-1) colon = index;
            index += 1;
        }
        if (colon == (unsigned __int64)-1 || colon == line_start ||
            index == line_start) return 0;
        name_length = colon - line_start;
        for (unsigned __int64 name_index = 0; name_index < name_length;
             name_index += 1) {
            if (!http_request_is_token((unsigned char)header_data[line_start + name_index])) {
                return 0;
            }
        }
        if (http_request_header_name_is_reserved(header_data + line_start,
                                                 name_length)) return 0;
        if (index == header_length) return 1;
        if (header_data[index] != '\r' || index + 1 >= header_length ||
            header_data[index + 1] != '\n') return 0;
        index += 2;
        if (index == header_length) return 0;
    }
    return 1;
}

unsigned __int64 http_request_write_header_block(
    const char *method_data, unsigned __int64 method_length,
    const char *target_data, unsigned __int64 target_length,
    const char *host_data, unsigned __int64 host_length,
    const char *header_data, unsigned __int64 header_length,
    const unsigned char *body_data, unsigned __int64 body_length,
    unsigned char *output_data, unsigned __int64 output_length) {
    static const unsigned char target_prefix[] = " HTTP/1.1\r\nHost: ";
    static const unsigned char header_prefix[] = "\r\n";
    static const unsigned char length_prefix[] = "\r\nContent-Length: ";
    static const unsigned char suffix[] = "\r\nConnection: close\r\n\r\n";
    unsigned char body_length_text[20];
    unsigned __int64 body_text_length;
    unsigned __int64 required = 0;
    unsigned __int64 offset = 0;
    unsigned __int64 index;
    if (method_data == 0 || method_length == 0 || target_data == 0 || target_length == 0 ||
        host_data == 0 || host_length == 0 || (header_data == 0 && header_length > 0) ||
        (body_data == 0 && body_length > 0) ||
        !http_request_header_block_is_valid(header_data, header_length)) return 0;
    for (index = 0; index < method_length; index += 1) {
        if (!http_request_is_token((unsigned char)method_data[index])) return 0;
    }
    for (index = 0; index < target_length; index += 1) {
        unsigned char value = (unsigned char)target_data[index];
        if (value < 0x21U || value > 0x7EU || value == '\r' || value == '\n') return 0;
    }
    for (index = 0; index < host_length; index += 1) {
        unsigned char value = (unsigned char)host_data[index];
        if (value < 0x21U || value > 0x7EU || value == '\r' || value == '\n') return 0;
    }
    body_text_length = format_uint(body_length, body_length_text, sizeof(body_length_text));
    if (body_text_length == 0 || !http_add_length(&required, method_length) ||
        !http_add_length(&required, 1) || !http_add_length(&required, target_length) ||
        !http_add_length(&required, sizeof(target_prefix) - 1) ||
        !http_add_length(&required, host_length) ||
        (header_length > 0 && (!http_add_length(&required, sizeof(header_prefix) - 1) ||
                               !http_add_length(&required, header_length))) ||
        !http_add_length(&required, sizeof(length_prefix) - 1) ||
        !http_add_length(&required, body_text_length) ||
        !http_add_length(&required, sizeof(suffix) - 1) ||
        !http_add_length(&required, body_length) || output_data == 0 ||
        output_length < required) return 0;
    http_copy_bytes(output_data, &offset, (const unsigned char *)method_data, method_length);
    http_copy_bytes(output_data, &offset, (const unsigned char *)" ", 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)target_data, target_length);
    http_copy_bytes(output_data, &offset, target_prefix, sizeof(target_prefix) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)host_data, host_length);
    if (header_length > 0) {
        http_copy_bytes(output_data, &offset, header_prefix, sizeof(header_prefix) - 1);
        http_copy_bytes(output_data, &offset, (const unsigned char *)header_data, header_length);
    }
    http_copy_bytes(output_data, &offset, length_prefix, sizeof(length_prefix) - 1);
    http_copy_bytes(output_data, &offset, body_length_text, body_text_length);
    http_copy_bytes(output_data, &offset, suffix, sizeof(suffix) - 1);
    http_copy_bytes(output_data, &offset, body_data, body_length);
    return required;
}

unsigned __int64 http_request_write_header_block_ex(
    const char *method_data, unsigned __int64 method_length,
    const char *target_data, unsigned __int64 target_length,
    const char *host_data, unsigned __int64 host_length,
    const char *header_data, unsigned __int64 header_length,
    int keep_alive,
    const unsigned char *body_data, unsigned __int64 body_length,
    unsigned char *output_data, unsigned __int64 output_length) {
    static const unsigned char target_prefix[] = " HTTP/1.1\r\nHost: ";
    static const unsigned char header_prefix[] = "\r\n";
    static const unsigned char length_prefix[] = "\r\nContent-Length: ";
    static const unsigned char close_suffix[] = "\r\nConnection: close\r\n\r\n";
    static const unsigned char keep_alive_suffix[] = "\r\nConnection: keep-alive\r\n\r\n";
    const unsigned char *suffix = keep_alive == 1 ? keep_alive_suffix : close_suffix;
    unsigned __int64 suffix_length = keep_alive == 1 ? sizeof(keep_alive_suffix) - 1 : sizeof(close_suffix) - 1;
    unsigned char body_length_text[20];
    unsigned __int64 body_text_length;
    unsigned __int64 required = 0;
    unsigned __int64 offset = 0;
    unsigned __int64 index;
    if (method_data == 0 || method_length == 0 || target_data == 0 || target_length == 0 ||
        host_data == 0 || host_length == 0 || (header_data == 0 && header_length > 0) ||
        (body_data == 0 && body_length > 0) ||
        !http_request_header_block_is_valid(header_data, header_length)) return 0;
    for (index = 0; index < method_length; index += 1) {
        if (!http_request_is_token((unsigned char)method_data[index])) return 0;
    }
    for (index = 0; index < target_length; index += 1) {
        unsigned char value = (unsigned char)target_data[index];
        if (value < 0x21U || value > 0x7EU || value == '\r' || value == '\n') return 0;
    }
    for (index = 0; index < host_length; index += 1) {
        unsigned char value = (unsigned char)host_data[index];
        if (value < 0x21U || value > 0x7EU || value == '\r' || value == '\n') return 0;
    }
    body_text_length = format_uint(body_length, body_length_text, sizeof(body_length_text));
    if (body_text_length == 0 || !http_add_length(&required, method_length) ||
        !http_add_length(&required, 1) || !http_add_length(&required, target_length) ||
        !http_add_length(&required, sizeof(target_prefix) - 1) ||
        !http_add_length(&required, host_length) ||
        (header_length > 0 && (!http_add_length(&required, sizeof(header_prefix) - 1) ||
                               !http_add_length(&required, header_length))) ||
        !http_add_length(&required, sizeof(length_prefix) - 1) ||
        !http_add_length(&required, body_text_length) ||
        !http_add_length(&required, suffix_length) ||
        !http_add_length(&required, body_length) || output_data == 0 ||
        output_length < required) return 0;
    http_copy_bytes(output_data, &offset, (const unsigned char *)method_data, method_length);
    http_copy_bytes(output_data, &offset, (const unsigned char *)" ", 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)target_data, target_length);
    http_copy_bytes(output_data, &offset, target_prefix, sizeof(target_prefix) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)host_data, host_length);
    if (header_length > 0) {
        http_copy_bytes(output_data, &offset, header_prefix, sizeof(header_prefix) - 1);
        http_copy_bytes(output_data, &offset, (const unsigned char *)header_data, header_length);
    }
    http_copy_bytes(output_data, &offset, length_prefix, sizeof(length_prefix) - 1);
    http_copy_bytes(output_data, &offset, body_length_text, body_text_length);
    http_copy_bytes(output_data, &offset, suffix, suffix_length);
    http_copy_bytes(output_data, &offset, body_data, body_length);
    return required;
}

unsigned __int64 http_request_write_header_ex(const char *method_data,
                                           unsigned __int64 method_length,
                                           const char *target_data,
                                           unsigned __int64 target_length,
                                           const char *host_data,
                                           unsigned __int64 host_length,
                                           const char *header_name_data,
                                           unsigned __int64 header_name_length,
                                           const char *header_value_data,
                                           unsigned __int64 header_value_length,
                                           int keep_alive,
                                           const unsigned char *body_data,
                                           unsigned __int64 body_length,
                                           unsigned char *output_data,
                                           unsigned __int64 output_length) {
    static const unsigned char target_prefix[] = " HTTP/1.1\r\nHost: ";
    static const unsigned char header_prefix[] = "\r\n";
    static const unsigned char header_separator[] = ": ";
    static const unsigned char length_prefix[] = "\r\nContent-Length: ";
    static const unsigned char close_suffix[] = "\r\nConnection: close\r\n\r\n";
    static const unsigned char keep_alive_suffix[] = "\r\nConnection: keep-alive\r\n\r\n";
    const unsigned char *suffix = keep_alive == 1 ? keep_alive_suffix : close_suffix;
    unsigned __int64 suffix_length = keep_alive == 1 ? sizeof(keep_alive_suffix) - 1 : sizeof(close_suffix) - 1;
    unsigned char body_length_text[20];
    unsigned __int64 body_text_length;
    unsigned __int64 required = 0;
    unsigned __int64 offset = 0;
    unsigned __int64 index;
    if (method_data == 0 || method_length == 0 || target_data == 0 || target_length == 0 ||
        host_data == 0 || host_length == 0 || header_name_data == 0 || header_name_length == 0 ||
        header_value_data == 0 && header_value_length > 0 ||
        body_data == 0 && body_length > 0 ||
        http_request_header_name_is_reserved(header_name_data, header_name_length)) return 0;
    for (index = 0; index < method_length; index += 1) {
        if (!http_request_is_token((unsigned char)method_data[index])) return 0;
    }
    for (index = 0; index < target_length; index += 1) {
        unsigned char value = (unsigned char)target_data[index];
        if (value < 0x21U || value > 0x7EU || value == '\r' || value == '\n') return 0;
    }
    for (index = 0; index < host_length; index += 1) {
        unsigned char value = (unsigned char)host_data[index];
        if (value < 0x21U || value > 0x7EU || value == '\r' || value == '\n') return 0;
    }
    for (index = 0; index < header_name_length; index += 1) {
        if (!http_request_is_token((unsigned char)header_name_data[index])) return 0;
    }
    for (index = 0; index < header_value_length; index += 1) {
        unsigned char value = (unsigned char)header_value_data[index];
        if (value < 0x20U || value > 0x7EU || value == '\r' || value == '\n') return 0;
    }
    body_text_length = format_uint(body_length, body_length_text, sizeof(body_length_text));
    if (body_text_length == 0 || !http_add_length(&required, method_length) ||
        !http_add_length(&required, 1) || !http_add_length(&required, target_length) ||
        !http_add_length(&required, sizeof(target_prefix) - 1) ||
        !http_add_length(&required, host_length) ||
        !http_add_length(&required, sizeof(header_prefix) - 1) ||
        !http_add_length(&required, header_name_length) ||
        !http_add_length(&required, sizeof(header_separator) - 1) ||
        !http_add_length(&required, header_value_length) ||
        !http_add_length(&required, sizeof(length_prefix) - 1) ||
        !http_add_length(&required, body_text_length) ||
        !http_add_length(&required, suffix_length) ||
        !http_add_length(&required, body_length) || output_data == 0 ||
        output_length < required) return 0;
    http_copy_bytes(output_data, &offset, (const unsigned char *)method_data, method_length);
    http_copy_bytes(output_data, &offset, (const unsigned char *)" ", 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)target_data, target_length);
    http_copy_bytes(output_data, &offset, target_prefix, sizeof(target_prefix) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)host_data, host_length);
    http_copy_bytes(output_data, &offset, header_prefix, sizeof(header_prefix) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)header_name_data, header_name_length);
    http_copy_bytes(output_data, &offset, header_separator, sizeof(header_separator) - 1);
    http_copy_bytes(output_data, &offset, (const unsigned char *)header_value_data, header_value_length);
    http_copy_bytes(output_data, &offset, length_prefix, sizeof(length_prefix) - 1);
    http_copy_bytes(output_data, &offset, body_length_text, body_text_length);
    http_copy_bytes(output_data, &offset, suffix, suffix_length);
    http_copy_bytes(output_data, &offset, body_data, body_length);
    return required;
}

unsigned __int64 http_request_write_header(const char *method_data,
                                           unsigned __int64 method_length,
                                           const char *target_data,
                                           unsigned __int64 target_length,
                                           const char *host_data,
                                           unsigned __int64 host_length,
                                           const char *header_name_data,
                                           unsigned __int64 header_name_length,
                                           const char *header_value_data,
                                           unsigned __int64 header_value_length,
                                           const unsigned char *body_data,
                                           unsigned __int64 body_length,
                                           unsigned char *output_data,
                                           unsigned __int64 output_length) {
    return http_request_write_header_ex(method_data, method_length,
                                        target_data, target_length,
                                        host_data, host_length,
                                        header_name_data, header_name_length,
                                        header_value_data, header_value_length,
                                        0, body_data, body_length,
                                        output_data, output_length);
}

unsigned __int64 http_request_write_cookie(const char *method_data,
                                           unsigned __int64 method_length,
                                           const char *target_data,
                                           unsigned __int64 target_length,
                                           const char *host_data,
                                           unsigned __int64 host_length,
                                           const char *cookie_name_data,
                                           unsigned __int64 cookie_name_length,
                                           const char *cookie_value_data,
                                           unsigned __int64 cookie_value_length,
                                           const unsigned char *body_data,
                                           unsigned __int64 body_length,
                                           unsigned char *output_data,
                                           unsigned __int64 output_length) {
    unsigned char cookie_data[4096];
    unsigned __int64 cookie_length = 0;
    unsigned __int64 index;
    if (cookie_name_data == 0 || cookie_name_length == 0 ||
        (cookie_value_data == 0 && cookie_value_length > 0)) return 0;
    for (index = 0; index < cookie_name_length; index += 1) {
        if (!http_cookie_name_is_token((unsigned char)cookie_name_data[index])) return 0;
    }
    for (index = 0; index < cookie_value_length; index += 1) {
        if (!http_cookie_value_is_octet((unsigned char)cookie_value_data[index])) return 0;
    }
    if (!http_add_length(&cookie_length, cookie_name_length) ||
        !http_add_length(&cookie_length, 1) ||
        !http_add_length(&cookie_length, cookie_value_length) ||
        cookie_length > sizeof(cookie_data)) return 0;
    index = 0;
    http_copy_bytes(cookie_data, &index, (const unsigned char *)cookie_name_data, cookie_name_length);
    http_copy_bytes(cookie_data, &index, (const unsigned char *)"=", 1);
    http_copy_bytes(cookie_data, &index, (const unsigned char *)cookie_value_data, cookie_value_length);
    return http_request_write_header(method_data, method_length,
                                     target_data, target_length,
                                     host_data, host_length,
                                     "Cookie", 6,
                                     (const char *)cookie_data, cookie_length,
                                     body_data, body_length,
                                     output_data, output_length);
}

unsigned __int64 http_request_write_cookie_block(
    const char *method_data, unsigned __int64 method_length,
    const char *target_data, unsigned __int64 target_length,
    const char *host_data, unsigned __int64 host_length,
    const char *cookie_block_data, unsigned __int64 cookie_block_length,
    const unsigned char *body_data, unsigned __int64 body_length,
    unsigned char *output_data, unsigned __int64 output_length) {
    unsigned char cookie_data[4096];
    unsigned __int64 cookie_length = 0;
    unsigned __int64 index = 0;
    int has_pair = 0;
    if (cookie_block_data == 0 || cookie_block_length == 0) return 0;
    while (index < cookie_block_length) {
        unsigned __int64 pair_start;
        unsigned __int64 pair_end;
        unsigned __int64 equals = (unsigned __int64)-1;
        unsigned __int64 name_start;
        unsigned __int64 name_end;
        unsigned __int64 value_start;
        unsigned __int64 value_end;
        unsigned __int64 name_length;
        unsigned __int64 value_length;
        unsigned __int64 pair_length;
        while (index < cookie_block_length &&
               (cookie_block_data[index] == ' ' || cookie_block_data[index] == '\t')) {
            index += 1;
        }
        if (index >= cookie_block_length) return 0;
        pair_start = index;
        while (index < cookie_block_length && cookie_block_data[index] != ';') {
            index += 1;
        }
        pair_end = index;
        while (pair_end > pair_start &&
               (cookie_block_data[pair_end - 1] == ' ' || cookie_block_data[pair_end - 1] == '\t')) {
            pair_end -= 1;
        }
        for (unsigned __int64 cursor = pair_start; cursor < pair_end; cursor += 1) {
            if (cookie_block_data[cursor] == '=' && equals == (unsigned __int64)-1) {
                equals = cursor;
            }
        }
        if (equals == (unsigned __int64)-1) return 0;
        name_start = pair_start;
        while (name_start < equals &&
               (cookie_block_data[name_start] == ' ' || cookie_block_data[name_start] == '\t')) {
            name_start += 1;
        }
        name_end = equals;
        while (name_end > name_start &&
               (cookie_block_data[name_end - 1] == ' ' || cookie_block_data[name_end - 1] == '\t')) {
            name_end -= 1;
        }
        value_start = equals + 1;
        while (value_start < pair_end &&
               (cookie_block_data[value_start] == ' ' || cookie_block_data[value_start] == '\t')) {
            value_start += 1;
        }
        value_end = pair_end;
        if (name_start == name_end || value_start > value_end) return 0;
        name_length = name_end - name_start;
        value_length = value_end - value_start;
        for (unsigned __int64 cursor = name_start; cursor < name_end; cursor += 1) {
            if (!http_cookie_name_is_token((unsigned char)cookie_block_data[cursor])) return 0;
        }
        for (unsigned __int64 cursor = value_start; cursor < value_end; cursor += 1) {
            if (!http_cookie_value_is_octet((unsigned char)cookie_block_data[cursor])) return 0;
        }
        pair_length = name_length;
        if (!http_add_length(&pair_length, 1) || !http_add_length(&pair_length, value_length)) return 0;
        if (has_pair && !http_add_length(&cookie_length, 2)) return 0;
        if (!http_add_length(&cookie_length, pair_length) || cookie_length > sizeof(cookie_data)) return 0;
        if (has_pair) {
            unsigned __int64 separator_offset = cookie_length - pair_length - 2;
            cookie_data[separator_offset] = ';';
            cookie_data[separator_offset + 1] = ' ';
        }
        {
            unsigned __int64 offset = cookie_length - pair_length;
            http_copy_bytes(cookie_data, &offset,
                            (const unsigned char *)cookie_block_data + name_start, name_length);
            http_copy_bytes(cookie_data, &offset, (const unsigned char *)"=", 1);
            http_copy_bytes(cookie_data, &offset,
                            (const unsigned char *)cookie_block_data + value_start, value_length);
        }
        has_pair = 1;
        if (index == cookie_block_length) break;
        index += 1;
        if (index == cookie_block_length) return 0;
    }
    return http_request_write_header(method_data, method_length,
                                     target_data, target_length,
                                     host_data, host_length,
                                     "Cookie", 6,
                                     (const char *)cookie_data, cookie_length,
                                     body_data, body_length,
                                     output_data, output_length);
}

static int http_request_is_token(unsigned char value) {
    return value >= 0x21 && value <= 0x7E &&
           value != '(' && value != ')' && value != '<' && value != '>' &&
           value != '@' && value != ',' && value != ';' && value != ':' &&
           value != '\\' && value != '"' && value != '/' && value != '[' &&
           value != ']' && value != '?' && value != '=' && value != '{' &&
           value != '}';
}

static unsigned char http_request_lower(unsigned char value) {
    return value >= 'A' && value <= 'Z' ? (unsigned char)(value + ('a' - 'A')) : value;
}

static int http_request_header_name_is_reserved(const char *name_data,
                                                unsigned __int64 name_length) {
    static const char *reserved[] = {"host", "content-length", "connection", "transfer-encoding"};
    static const unsigned __int64 lengths[] = {4, 14, 10, 17};
    unsigned __int64 candidate;
    unsigned __int64 index;
    if (name_data == 0 || name_length == 0) return 1;
    for (candidate = 0; candidate < 4; candidate += 1) {
        if (name_length != lengths[candidate]) continue;
        for (index = 0; index < name_length; index += 1) {
            if (http_request_lower((unsigned char)name_data[index]) !=
                (unsigned char)reserved[candidate][index]) break;
        }
        if (index == name_length) return 1;
    }
    return 0;
}

static int http_request_header_name_equals(const unsigned char *data,
                                           unsigned __int64 start,
                                           unsigned __int64 length,
                                           const char *name_data,
                                           unsigned __int64 name_length) {
    unsigned __int64 index;
    if (data == 0 || name_data == 0 || length != name_length) {
        return 0;
    }
    for (index = 0; index < length; index += 1) {
        if (http_request_lower(data[start + index]) !=
            http_request_lower((unsigned char)name_data[index])) {
            return 0;
        }
    }
    return 1;
}

static int http_request_parse_line(const unsigned char *data,
                                   unsigned __int64 length,
                                   unsigned __int64 *method_start,
                                   unsigned __int64 *method_length,
                                   unsigned __int64 *target_start,
                                   unsigned __int64 *target_length,
                                   unsigned __int64 *headers_start) {
    unsigned __int64 line_end = 0;
    unsigned __int64 first_space = 0;
    unsigned __int64 second_space = 0;
    unsigned __int64 index;
    if (data == 0 || length == 0 || method_start == 0 || method_length == 0 ||
        target_start == 0 || target_length == 0 || headers_start == 0) {
        return 0;
    }
    for (index = 0; index + 1 < length; index += 1) {
        if (data[index] == '\r' && data[index + 1] == '\n') {
            line_end = index;
            break;
        }
    }
    if (line_end == 0) {
        return 0;
    }
    for (index = 0; index < line_end; index += 1) {
        if (data[index] == ' ') {
            first_space = index;
            break;
        }
    }
    if (first_space == 0 || first_space + 1 >= line_end) {
        return 0;
    }
    for (index = first_space + 1; index < line_end; index += 1) {
        if (data[index] == ' ') {
            second_space = index;
            break;
        }
    }
    if (second_space == 0 || second_space + 1 >= line_end ||
        line_end - (second_space + 1) != 8) {
        return 0;
    }
    if (data[second_space + 1] != 'H' || data[second_space + 2] != 'T' ||
        data[second_space + 3] != 'T' || data[second_space + 4] != 'P' ||
        data[second_space + 5] != '/' || data[second_space + 6] != '1' ||
        data[second_space + 7] != '.' || data[second_space + 8] != '1') {
        return 0;
    }
    for (index = 0; index < first_space; index += 1) {
        if (!http_request_is_token(data[index])) {
            return 0;
        }
    }
    for (index = first_space + 1; index < second_space; index += 1) {
        if (data[index] < 0x21 || data[index] > 0x7E) {
            return 0;
        }
    }
    *method_start = 0;
    *method_length = first_space;
    *target_start = first_space + 1;
    *target_length = second_space - first_space - 1;
    *headers_start = line_end + 2;
    return *method_length > 0 && *target_length > 0;
}

static int http_request_next_header(const unsigned char *data,
                                    unsigned __int64 length,
                                    unsigned __int64 cursor,
                                    unsigned __int64 *next_cursor,
                                    unsigned __int64 *name_start,
                                    unsigned __int64 *name_length,
                                    unsigned __int64 *value_start,
                                    unsigned __int64 *value_length) {
    unsigned __int64 line_end;
    unsigned __int64 colon = 0;
    unsigned __int64 index;
    if (data == 0 || next_cursor == 0 || name_start == 0 || name_length == 0 ||
        value_start == 0 || value_length == 0 || cursor > length) {
        return -1;
    }
    if (cursor + 2 <= length && data[cursor] == '\r' && data[cursor + 1] == '\n') {
        *next_cursor = cursor + 2;
        return 0;
    }
    line_end = cursor;
    for (index = cursor; index + 1 < length; index += 1) {
        if (data[index] == '\r' && data[index + 1] == '\n') {
            line_end = index;
            break;
        }
    }
    if (line_end == cursor || line_end == length) {
        return -1;
    }
    for (index = cursor; index < line_end; index += 1) {
        if (data[index] == ':') {
            colon = index;
            break;
        }
    }
    if (colon == cursor) {
        return -1;
    }
    for (index = cursor; index < colon; index += 1) {
        if (!http_request_is_token(data[index])) {
            return -1;
        }
    }
    *name_start = cursor;
    *name_length = colon - cursor;
    *value_start = colon + 1;
    *value_length = line_end - colon - 1;
    while (*value_length > 0 &&
           (data[*value_start] == ' ' || data[*value_start] == '\t')) {
        *value_start += 1;
        *value_length -= 1;
    }
    while (*value_length > 0 &&
           (data[*value_start + *value_length - 1] == ' ' ||
            data[*value_start + *value_length - 1] == '\t')) {
        *value_length -= 1;
    }
    *next_cursor = line_end + 2;
    return 1;
}

static int http_request_headers(const unsigned char *data,
                                unsigned __int64 length,
                                unsigned __int64 headers_start,
                                unsigned __int64 *body_start,
                                unsigned __int64 *content_length,
                                int *has_content_length,
                                int *has_transfer_encoding) {
    unsigned __int64 cursor = headers_start;
    unsigned __int64 next_cursor;
    unsigned __int64 name_start;
    unsigned __int64 name_length;
    unsigned __int64 value_start;
    unsigned __int64 value_length;
    unsigned __int64 parsed_length;
    unsigned __int64 index;
    int result;
    if (data == 0 || body_start == 0 || content_length == 0 ||
        has_content_length == 0 || has_transfer_encoding == 0) {
        return 0;
    }
    *content_length = 0;
    *has_content_length = 0;
    *has_transfer_encoding = 0;
    for (;;) {
        result = http_request_next_header(data, length, cursor, &next_cursor,
                                          &name_start, &name_length,
                                          &value_start, &value_length);
        if (result < 0) {
            return 0;
        }
        if (result == 0) {
            *body_start = next_cursor;
            return 1;
        }
        if (http_request_header_name_equals(data, name_start, name_length,
                                            "Content-Length", 14)) {
            if (*has_content_length || value_length == 0) {
                return 0;
            }
            parsed_length = 0;
            for (index = 0; index < value_length; index += 1) {
                unsigned char digit = data[value_start + index];
                if (digit < '0' || digit > '9' ||
                    parsed_length > (((unsigned __int64)-1) - (digit - '0')) / 10) {
                    return 0;
                }
                parsed_length = parsed_length * 10 + (digit - '0');
            }
            *content_length = parsed_length;
            *has_content_length = 1;
        } else if (http_request_header_name_equals(data, name_start, name_length,
                                                   "Transfer-Encoding", 17)) {
            *has_transfer_encoding = 1;
        }
        cursor = next_cursor;
    }
}

static int http_request_find_header(const unsigned char *data,
                                    unsigned __int64 length,
                                    unsigned __int64 headers_start,
                                    const char *name_data,
                                    unsigned __int64 name_length,
                                    unsigned __int64 *value_start,
                                    unsigned __int64 *value_length) {
    unsigned __int64 cursor = headers_start;
    unsigned __int64 next_cursor;
    unsigned __int64 name_start;
    unsigned __int64 current_name_length;
    unsigned __int64 current_value_start;
    unsigned __int64 current_value_length;
    int result;
    int found = 0;
    if (name_data == 0 || name_length == 0 || value_start == 0 || value_length == 0) {
        return -1;
    }
    for (;;) {
        result = http_request_next_header(data, length, cursor, &next_cursor,
                                          &name_start, &current_name_length,
                                          &current_value_start, &current_value_length);
        if (result < 0) {
            return -1;
        }
        if (result == 0) {
            return found;
        }
        if (http_request_header_name_equals(data, name_start, current_name_length,
                                            name_data, name_length)) {
            if (found) {
                return -1;
            }
            found = 1;
            *value_start = current_value_start;
            *value_length = current_value_length;
        }
        cursor = next_cursor;
    }
}

static int http_response_parse_line(const unsigned char *data,
                                    unsigned __int64 length,
                                    unsigned short *status,
                                    unsigned __int64 *headers_start) {
    static const unsigned char prefix[] = "HTTP/1.1 ";
    unsigned __int64 line_end = 0;
    unsigned __int64 index;
    if (data == 0 || status == 0 || headers_start == 0 || length < 12) {
        return 0;
    }
    for (index = 0; index + 1 < length; index += 1) {
        if (data[index] == '\r' && data[index + 1] == '\n') {
            line_end = index;
            break;
        }
    }
    if (line_end < 12) {
        return 0;
    }
    for (index = 0; index < sizeof(prefix) - 1; index += 1) {
        if (data[index] != prefix[index]) {
            return 0;
        }
    }
    if (data[9] < '0' || data[9] > '9' || data[10] < '0' || data[10] > '9' ||
        data[11] < '0' || data[11] > '9' || (line_end > 12 && data[12] != ' ')) {
        return 0;
    }
    for (index = 13; index < line_end; index += 1) {
        if (data[index] < 0x20 && data[index] != '\t') {
            return 0;
        }
    }
    *status = (unsigned short)((data[9] - '0') * 100 +
                               (data[10] - '0') * 10 + (data[11] - '0'));
    *headers_start = line_end + 2;
    return 1;
}

static unsigned __int64 http_response_copy_field(const unsigned char *input_data,
                                                  unsigned __int64 field_start,
                                                  unsigned __int64 field_length,
                                                  unsigned char *output_data,
                                                  unsigned __int64 output_length) {
    unsigned __int64 index;
    if (field_length == 0 || output_data == 0 || output_length < field_length) {
        return 0;
    }
    for (index = 0; index < field_length; index += 1) {
        output_data[index] = input_data[field_start + index];
    }
    return field_length;
}

unsigned short http_response_status_prefix(const unsigned char *input_data,
                                           unsigned __int64 input_capacity,
                                           unsigned __int64 input_length) {
    unsigned short status;
    unsigned __int64 headers_start;
    if (input_length > input_capacity ||
        !http_response_parse_line(input_data, input_length, &status, &headers_start)) {
        return 0;
    }
    return status;
}

unsigned short http_response_status(const unsigned char *input_data,
                                    unsigned __int64 input_length) {
    return http_response_status_prefix(input_data, input_length, input_length);
}

unsigned __int64 http_response_header_prefix(const unsigned char *input_data,
                                             unsigned __int64 input_capacity,
                                             unsigned __int64 input_length,
                                             const char *name_data,
                                             unsigned __int64 name_length,
                                             unsigned char *output_data,
                                             unsigned __int64 output_length) {
    unsigned short status;
    unsigned __int64 headers_start;
    unsigned __int64 value_start;
    unsigned __int64 value_length;
    if (input_length > input_capacity ||
        !http_response_parse_line(input_data, input_length, &status, &headers_start) ||
        http_request_find_header(input_data, input_length, headers_start,
                                 name_data, name_length, &value_start,
                                 &value_length) <= 0) {
        return 0;
    }
    return http_response_copy_field(input_data, value_start, value_length,
                                    output_data, output_length);
}

unsigned __int64 http_response_header(const unsigned char *input_data,
                                      unsigned __int64 input_length,
                                      const char *name_data,
                                      unsigned __int64 name_length,
                                      unsigned char *output_data,
                                      unsigned __int64 output_length) {
    return http_response_header_prefix(input_data, input_length, input_length,
                                       name_data, name_length, output_data,
                                       output_length);
}

int http_response_header_exact(const unsigned char *input_data,
                               unsigned __int64 input_length,
                               const char *name_data,
                               unsigned __int64 name_length,
                               unsigned char *output_data,
                               unsigned __int64 output_capacity,
                               unsigned __int64 *length_data,
                               unsigned __int64 length_capacity) {
    unsigned short status;
    unsigned __int64 headers_start;
    unsigned __int64 value_start;
    unsigned __int64 value_length;
    if (length_data == 0 || length_capacity == 0 ||
        !http_response_parse_line(input_data, input_length, &status, &headers_start) ||
        http_request_find_header(input_data, input_length, headers_start,
                                 name_data, name_length, &value_start,
                                 &value_length) <= 0 ||
        value_length > output_capacity ||
        (value_length > 0 && output_data == 0)) {
        return 0;
    }
    if (value_length > 0) {
        http_response_copy_field(input_data, value_start, value_length,
                                 output_data, output_capacity);
    }
    length_data[0] = value_length;
    return 1;
}

unsigned __int64 http_response_body_prefix(const unsigned char *input_data,
                                           unsigned __int64 input_capacity,
                                           unsigned __int64 input_length,
                                           unsigned char *output_data,
                                           unsigned __int64 output_length) {
    unsigned short status;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    int has_content_length;
    int has_transfer_encoding;
    if (input_length > input_capacity ||
        !http_response_parse_line(input_data, input_length, &status, &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start, &body_start,
                              &content_length, &has_content_length,
                              &has_transfer_encoding) || !has_content_length ||
        has_transfer_encoding || body_start > input_length ||
        content_length > input_length - body_start ||
        (content_length > 0 && output_data == 0) || output_length < content_length) {
        return 0;
    }
    return http_response_copy_field(input_data, body_start, content_length,
                                    output_data, output_length);
}

unsigned __int64 http_response_body(const unsigned char *input_data,
                                    unsigned __int64 input_length,
                                    unsigned char *output_data,
                                    unsigned __int64 output_length) {
    return http_response_body_prefix(input_data, input_length, input_length,
                                     output_data, output_length);
}

int http_response_body_exact(const unsigned char *input_data,
                             unsigned __int64 input_length,
                             unsigned char *output_data,
                             unsigned __int64 output_capacity,
                             unsigned __int64 *length_data,
                             unsigned __int64 length_capacity) {
    unsigned short status;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    int has_content_length;
    int has_transfer_encoding;
    if (length_data == 0 || length_capacity == 0 ||
        !http_response_parse_line(input_data, input_length, &status, &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start, &body_start,
                              &content_length, &has_content_length,
                              &has_transfer_encoding) || !has_content_length ||
        has_transfer_encoding || body_start > input_length ||
        content_length > input_length - body_start ||
        content_length > output_capacity ||
        (content_length > 0 && output_data == 0)) {
        return 0;
    }
    if (content_length > 0) {
        http_response_copy_field(input_data, body_start, content_length,
                                 output_data, output_capacity);
    }
    length_data[0] = content_length;
    return 1;
}

static int http_response_chunked_scan(const unsigned char *input_data,
                                      unsigned __int64 input_length,
                                      unsigned __int64 body_start,
                                      unsigned char *output_data,
                                      unsigned __int64 output_capacity,
                                      unsigned __int64 *body_length,
                                      unsigned __int64 *body_end) {
    unsigned __int64 cursor = body_start;
    unsigned __int64 total = 0;
    unsigned __int64 line_end;
    unsigned __int64 size_end;
    unsigned __int64 chunk_size;
    unsigned __int64 index;
    unsigned __int64 next_cursor;
    unsigned __int64 trailer_name_start;
    unsigned __int64 trailer_name_length;
    unsigned __int64 trailer_value_start;
    unsigned __int64 trailer_value_length;
    int result;
    if (input_data == 0 || body_length == 0 || body_end == 0 || body_start > input_length) {
        return 0;
    }
    for (;;) {
        line_end = cursor;
        while (line_end + 1 < input_length &&
               !(input_data[line_end] == '\r' && input_data[line_end + 1] == '\n')) {
            line_end += 1;
        }
        if (line_end + 1 >= input_length || line_end == cursor) {
            return 0;
        }
        size_end = cursor;
        while (size_end < line_end && input_data[size_end] != ';') {
            size_end += 1;
        }
        if (size_end == cursor) return 0;
        chunk_size = 0;
        for (index = cursor; index < size_end; index += 1) {
            unsigned char digit = input_data[index];
            unsigned __int64 value;
            if (digit >= '0' && digit <= '9') value = (unsigned __int64)(digit - '0');
            else if (digit >= 'a' && digit <= 'f') value = (unsigned __int64)(digit - 'a' + 10);
            else if (digit >= 'A' && digit <= 'F') value = (unsigned __int64)(digit - 'A' + 10);
            else return 0;
            if (chunk_size > (((unsigned __int64)-1) - value) / 16) return 0;
            chunk_size = chunk_size * 16 + value;
        }
        cursor = line_end + 2;
        if (chunk_size == 0) {
            for (;;) {
                result = http_request_next_header(input_data, input_length, cursor,
                                                  &next_cursor, &trailer_name_start,
                                                  &trailer_name_length, &trailer_value_start,
                                                  &trailer_value_length);
                if (result < 0) return 0;
                if (result == 0) {
                    *body_length = total;
                    *body_end = next_cursor;
                    return 1;
                }
                cursor = next_cursor;
            }
        }
        if (chunk_size > input_length - cursor || total > (((unsigned __int64)-1) - chunk_size)) {
            return 0;
        }
        if (output_data != 0) {
            for (index = 0; index < chunk_size; index += 1) {
                output_data[total + index] = input_data[cursor + index];
            }
        }
        total += chunk_size;
        cursor += chunk_size;
        if (cursor + 1 >= input_length || input_data[cursor] != '\r' || input_data[cursor + 1] != '\n') {
            return 0;
        }
        cursor += 2;
        if (total > output_capacity && output_data != 0) return 0;
    }
}

int http_response_body_chunked_exact(const unsigned char *input_data,
                                     unsigned __int64 input_length,
                                     unsigned char *output_data,
                                     unsigned __int64 output_capacity,
                                     unsigned __int64 *length_data,
                                     unsigned __int64 length_capacity) {
    unsigned short status;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    unsigned __int64 transfer_start;
    unsigned __int64 transfer_length;
    unsigned __int64 body_length;
    unsigned __int64 body_end;
    int has_content_length;
    int has_transfer_encoding;
    if (length_data == 0 || length_capacity == 0 ||
        !http_response_parse_line(input_data, input_length, &status, &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start, &body_start,
                              &content_length, &has_content_length,
                              &has_transfer_encoding) || !has_transfer_encoding ||
        has_content_length ||
        http_request_find_header(input_data, input_length, headers_start,
                                 "Transfer-Encoding", 17, &transfer_start,
                                 &transfer_length) != 1 || transfer_length != 7 ||
        input_data[transfer_start] != 'c' && input_data[transfer_start] != 'C') {
        return 0;
    }
    if (transfer_length != 7 ||
        !((input_data[transfer_start + 1] == 'h' || input_data[transfer_start + 1] == 'H') &&
          (input_data[transfer_start + 2] == 'u' || input_data[transfer_start + 2] == 'U') &&
          (input_data[transfer_start + 3] == 'n' || input_data[transfer_start + 3] == 'N') &&
          (input_data[transfer_start + 4] == 'k' || input_data[transfer_start + 4] == 'K') &&
          (input_data[transfer_start + 5] == 'e' || input_data[transfer_start + 5] == 'E') &&
          (input_data[transfer_start + 6] == 'd' || input_data[transfer_start + 6] == 'D'))) {
        return 0;
    }
    if (!http_response_chunked_scan(input_data, input_length, body_start, 0, 0,
                                    &body_length, &body_end) || body_end != input_length ||
        body_length > output_capacity || (body_length > 0 && output_data == 0)) {
        return 0;
    }
    if (body_length > 0 &&
        !http_response_chunked_scan(input_data, input_length, body_start, output_data,
                                    output_capacity, &body_length, &body_end)) {
        return 0;
    }
    length_data[0] = body_length;
    return 1;
}

int http_request_body_chunked_exact(const unsigned char *input_data,
                                    unsigned __int64 input_length,
                                    unsigned char *output_data,
                                    unsigned __int64 output_capacity,
                                    unsigned __int64 *length_data,
                                    unsigned __int64 length_capacity) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    unsigned __int64 transfer_start;
    unsigned __int64 transfer_length;
    unsigned __int64 body_length;
    unsigned __int64 body_end;
    int has_content_length;
    int has_transfer_encoding;
    if (length_data == 0 || length_capacity == 0 ||
        !http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start, &body_start,
                              &content_length, &has_content_length,
                              &has_transfer_encoding) || !has_transfer_encoding ||
        has_content_length ||
        http_request_find_header(input_data, input_length, headers_start,
                                 "Transfer-Encoding", 17, &transfer_start,
                                 &transfer_length) != 1 || transfer_length != 7 ||
        transfer_start > input_length - transfer_length) {
        return 0;
    }
    if (!((input_data[transfer_start] == 'c' || input_data[transfer_start] == 'C') &&
          (input_data[transfer_start + 1] == 'h' || input_data[transfer_start + 1] == 'H') &&
          (input_data[transfer_start + 2] == 'u' || input_data[transfer_start + 2] == 'U') &&
          (input_data[transfer_start + 3] == 'n' || input_data[transfer_start + 3] == 'N') &&
          (input_data[transfer_start + 4] == 'k' || input_data[transfer_start + 4] == 'K') &&
          (input_data[transfer_start + 5] == 'e' || input_data[transfer_start + 5] == 'E') &&
          (input_data[transfer_start + 6] == 'd' || input_data[transfer_start + 6] == 'D'))) {
        return 0;
    }
    if (!http_response_chunked_scan(input_data, input_length, body_start, 0, 0,
                                    &body_length, &body_end) || body_end != input_length ||
        body_length > output_capacity || (body_length > 0 && output_data == 0)) {
        return 0;
    }
    if (body_length > 0 &&
        !http_response_chunked_scan(input_data, input_length, body_start, output_data,
                                    output_capacity, &body_length, &body_end)) {
        return 0;
    }
    length_data[0] = body_length;
    return 1;
}

int http_request_keep_alive(const unsigned char *input_data,
                            unsigned __int64 input_length) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    unsigned __int64 value_start;
    unsigned __int64 value_length;
    unsigned __int64 index;
    int header_result;
    int has_content_length;
    int has_transfer_encoding;
    static const unsigned char close_value[] = "close";
    static const unsigned char keep_value[] = "keep-alive";
    if (!http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start, &body_start,
                              &content_length, &has_content_length,
                              &has_transfer_encoding)) {
        return 0;
    }
    header_result = http_request_find_header(input_data, input_length, headers_start,
                                             "Connection", 10, &value_start,
                                             &value_length);
    if (header_result < 0) {
        return 0;
    }
    if (header_result == 0) {
        return 1;
    }
    if (value_length == sizeof(close_value) - 1) {
        for (index = 0; index < value_length; index += 1) {
            if (http_request_lower(input_data[value_start + index]) != close_value[index]) {
                return 0;
            }
        }
        return 0;
    }
    if (value_length == sizeof(keep_value) - 1) {
        for (index = 0; index < value_length; index += 1) {
            if (http_request_lower(input_data[value_start + index]) != keep_value[index]) {
                return 0;
            }
        }
        return 1;
    }
    return 0;
}

int http_request_is_complete(const unsigned char *input_data,
                             unsigned __int64 input_length) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    int has_content_length;
    int has_transfer_encoding;
    if (!http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start, &body_start,
                              &content_length, &has_content_length,
                              &has_transfer_encoding) || has_transfer_encoding) {
        return 0;
    }
    if (!has_content_length) {
        return body_start == input_length;
    }
    return body_start <= input_length && input_length - body_start >= content_length;
}

int http_request_is_complete_prefix(const unsigned char *input_data,
                                    unsigned __int64 input_capacity,
                                    unsigned __int64 input_length) {
    if (input_length > input_capacity) return 0;
    return http_request_is_complete(input_data, input_length);
}

unsigned __int64 http_request_append(
    unsigned char *output_data, unsigned __int64 output_length,
    unsigned __int64 current_length, const unsigned char *chunk_data,
    unsigned __int64 chunk_length, unsigned __int64 append_length) {
    unsigned __int64 index;
    if (output_data == 0 || current_length > output_length ||
        append_length > chunk_length ||
        (append_length > 0 && chunk_data == 0) ||
        append_length > output_length - current_length) {
        return 0;
    }
    for (index = 0; index < append_length; index += 1) {
        output_data[current_length + index] = chunk_data[index];
    }
    return current_length + append_length;
}

unsigned __int64 http_request_frame_length_prefix(
    const unsigned char *input_data,
    unsigned __int64 input_capacity,
    unsigned __int64 input_length) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    int has_content_length;
    int has_transfer_encoding;
    if (input_length > input_capacity ||
        !http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start, &body_start,
                              &content_length, &has_content_length,
                              &has_transfer_encoding) || has_transfer_encoding) {
        return 0;
    }
    if (!has_content_length) {
        return body_start;
    }
    if (body_start > input_length || content_length > input_length - body_start) {
        return 0;
    }
    return body_start + content_length;
}

unsigned __int64 http_request_chunked_frame_length_prefix(
    const unsigned char *input_data,
    unsigned __int64 input_capacity,
    unsigned __int64 input_length) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    unsigned __int64 transfer_start;
    unsigned __int64 transfer_length;
    unsigned __int64 body_length;
    unsigned __int64 body_end;
    int has_content_length;
    int has_transfer_encoding;
    if (input_length > input_capacity ||
        !http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start, &body_start,
                              &content_length, &has_content_length,
                              &has_transfer_encoding) || !has_transfer_encoding ||
        has_content_length ||
        http_request_find_header(input_data, input_length, headers_start,
                                 "Transfer-Encoding", 17, &transfer_start,
                                 &transfer_length) != 1 || transfer_length != 7 ||
        input_length < transfer_length ||
        transfer_start > input_length - transfer_length) {
        return 0;
    }
    if (!((input_data[transfer_start] == 'c' || input_data[transfer_start] == 'C') &&
          (input_data[transfer_start + 1] == 'h' || input_data[transfer_start + 1] == 'H') &&
          (input_data[transfer_start + 2] == 'u' || input_data[transfer_start + 2] == 'U') &&
          (input_data[transfer_start + 3] == 'n' || input_data[transfer_start + 3] == 'N') &&
          (input_data[transfer_start + 4] == 'k' || input_data[transfer_start + 4] == 'K') &&
          (input_data[transfer_start + 5] == 'e' || input_data[transfer_start + 5] == 'E') &&
          (input_data[transfer_start + 6] == 'd' || input_data[transfer_start + 6] == 'D'))) {
        return 0;
    }
    if (!http_response_chunked_scan(input_data, input_length, body_start, 0, 0,
                                    &body_length, &body_end)) {
        return 0;
    }
    return body_end;
}

unsigned __int64 http_request_consume_prefix(
    unsigned char *input_data,
    unsigned __int64 input_capacity,
    unsigned __int64 input_length,
    unsigned __int64 consumed_length) {
    unsigned __int64 index;
    if (input_data == 0 || input_length > input_capacity ||
        consumed_length > input_length) {
        return 0;
    }
    for (index = consumed_length; index < input_length; index += 1) {
        input_data[index - consumed_length] = input_data[index];
    }
    return input_length - consumed_length;
}

static unsigned __int64 http_request_copy_field(const unsigned char *input_data,
                                                unsigned __int64 field_start,
                                                unsigned __int64 field_length,
                                                unsigned char *output_data,
                                                unsigned __int64 output_length) {
    unsigned __int64 index;
    if (field_length == 0 || output_data == 0 || output_length < field_length) {
        return 0;
    }
    for (index = 0; index < field_length; index += 1) {
        output_data[index] = input_data[field_start + index];
    }
    return field_length;
}

unsigned __int64 http_request_method(const unsigned char *input_data,
                                     unsigned __int64 input_length,
                                     unsigned char *output_data,
                                     unsigned __int64 output_length) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    if (!http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start)) {
        return 0;
    }
    return http_request_copy_field(input_data, method_start, method_length,
                                   output_data, output_length);
}

unsigned __int64 http_request_target(const unsigned char *input_data,
                                     unsigned __int64 input_length,
                                     unsigned char *output_data,
                                     unsigned __int64 output_length) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    if (!http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start)) {
        return 0;
    }
    return http_request_copy_field(input_data, target_start, target_length,
                                   output_data, output_length);
}

static int http_target_hex_value(unsigned char value) {
    if (value >= '0' && value <= '9') return (int)(value - '0');
    if (value >= 'a' && value <= 'f') return (int)(value - 'a' + 10);
    if (value >= 'A' && value <= 'F') return (int)(value - 'A' + 10);
    return -1;
}

int http_request_target_decode_exact(const unsigned char *input_data,
                                     unsigned __int64 input_length,
                                     unsigned char *output_data,
                                     unsigned __int64 output_capacity,
                                     unsigned __int64 *length_data,
                                     unsigned __int64 length_capacity) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 index;
    unsigned __int64 output_index = 0;
    unsigned __int64 decoded_length = 0;
    if (length_data == 0 || length_capacity == 0 ||
        !http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) || target_length == 0) {
        return 0;
    }
    for (index = 0; index < target_length; index += 1) {
        unsigned char value = input_data[target_start + index];
        if (value == '%') {
            int high;
            int low;
            unsigned char decoded;
            if (index + 2 >= target_length ||
                (high = http_target_hex_value(input_data[target_start + index + 1])) < 0 ||
                (low = http_target_hex_value(input_data[target_start + index + 2])) < 0) {
                return 0;
            }
            decoded = (unsigned char)(high * 16 + low);
            if (decoded == 0) return 0;
            index += 2;
        }
        if (decoded_length == (unsigned __int64)-1) return 0;
        decoded_length += 1;
    }
    if (decoded_length > output_capacity ||
        (decoded_length > 0 && output_data == 0)) {
        return 0;
    }
    for (index = 0; index < target_length;) {
        unsigned char value = input_data[target_start + index];
        if (value == '%') {
            int high = http_target_hex_value(input_data[target_start + index + 1]);
            int low = http_target_hex_value(input_data[target_start + index + 2]);
            output_data[output_index] = (unsigned char)(high * 16 + low);
            index += 3;
        } else {
            output_data[output_index] = value;
            index += 1;
        }
        output_index += 1;
    }
    length_data[0] = decoded_length;
    return 1;
}

unsigned __int64 http_request_header(const unsigned char *input_data,
                                     unsigned __int64 input_length,
                                     const char *name_data,
                                     unsigned __int64 name_length,
                                     unsigned char *output_data,
                                     unsigned __int64 output_length) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 value_start;
    unsigned __int64 value_length;
    if (!http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        http_request_find_header(input_data, input_length, headers_start,
                                 name_data, name_length, &value_start,
                                 &value_length) <= 0) {
        return 0;
    }
    return http_request_copy_field(input_data, value_start, value_length,
                                   output_data, output_length);
}

int http_request_header_exact(const unsigned char *input_data,
                              unsigned __int64 input_length,
                              const char *name_data,
                              unsigned __int64 name_length,
                              unsigned char *output_data,
                              unsigned __int64 output_capacity,
                              unsigned __int64 *length_data,
                              unsigned __int64 length_capacity) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 value_start;
    unsigned __int64 value_length;
    if (length_data == 0 || length_capacity == 0 ||
        !http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        http_request_find_header(input_data, input_length, headers_start,
                                 name_data, name_length, &value_start,
                                 &value_length) <= 0 ||
        value_length > output_capacity ||
        (value_length > 0 && output_data == 0)) {
        return 0;
    }
    if (value_length > 0) {
        http_request_copy_field(input_data, value_start, value_length,
                                output_data, output_capacity);
    }
    length_data[0] = value_length;
    return 1;
}

unsigned __int64 http_request_body(const unsigned char *input_data,
                                   unsigned __int64 input_length,
                                   unsigned char *output_data,
                                   unsigned __int64 output_length) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    int has_content_length;
    int has_transfer_encoding;
    if (!http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start, &body_start,
                              &content_length, &has_content_length,
                              &has_transfer_encoding) || !has_content_length ||
        has_transfer_encoding || body_start > input_length ||
        input_length - body_start < content_length ||
        (content_length > 0 && output_data == 0) || output_length < content_length) {
        return 0;
    }
    return http_request_copy_field(input_data, body_start, content_length,
                                   output_data, output_length);
}

int http_request_body_exact(const unsigned char *input_data,
                            unsigned __int64 input_length,
                            unsigned char *output_data,
                            unsigned __int64 output_capacity,
                            unsigned __int64 *length_data,
                            unsigned __int64 length_capacity) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    int has_content_length;
    int has_transfer_encoding;
    if (length_data == 0 || length_capacity == 0 ||
        !http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start,
                              &body_start, &content_length,
                              &has_content_length, &has_transfer_encoding) ||
        !has_content_length || has_transfer_encoding || body_start > input_length ||
        content_length > input_length - body_start ||
        content_length > output_capacity ||
        (content_length > 0 && output_data == 0)) {
        return 0;
    }
    if (content_length > 0) {
        http_request_copy_field(input_data, body_start, content_length,
                                output_data, output_capacity);
    }
    length_data[0] = content_length;
    return 1;
}

int http_request_body_exact_prefix(const unsigned char *input_data,
                                   unsigned __int64 input_capacity,
                                   unsigned __int64 input_length,
                                   unsigned char *output_data,
                                   unsigned __int64 output_capacity,
                                   unsigned __int64 *length_data,
                                   unsigned __int64 length_capacity) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    int has_content_length;
    int has_transfer_encoding;
    if (length_data == 0 || length_capacity == 0 || input_length > input_capacity ||
        !http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start,
                              &body_start, &content_length,
                              &has_content_length, &has_transfer_encoding) ||
        !has_content_length || has_transfer_encoding || body_start > input_length ||
        content_length > input_length - body_start ||
        content_length > output_capacity ||
        (content_length > 0 && output_data == 0)) {
        return 0;
    }
    if (content_length > 0) {
        http_request_copy_field(input_data, body_start, content_length,
                                output_data, output_capacity);
    }
    length_data[0] = content_length;
    return 1;
}

static int http_query_hex_value(unsigned char value) {
    if (value >= '0' && value <= '9') return (int)(value - '0');
    if (value >= 'a' && value <= 'f') return (int)(value - 'a' + 10);
    if (value >= 'A' && value <= 'F') return (int)(value - 'A' + 10);
    return -1;
}

unsigned __int64 http_query_param(const unsigned char *input_data,
                                  unsigned __int64 input_length,
                                  const char *key_data,
                                  unsigned __int64 key_length,
                                  unsigned char *output_data,
                                  unsigned __int64 output_length) {
    unsigned __int64 query_start = 0;
    unsigned __int64 cursor;
    unsigned __int64 value_start = 0;
    unsigned __int64 value_length = 0;
    unsigned __int64 decoded_length = 0;
    unsigned __int64 output_offset = 0;
    unsigned __int64 result_length = 0;
    int matched = 0;
    if (input_data == 0 || key_data == 0 || key_length == 0 || output_data == 0) return 0;
    for (cursor = 0; cursor < input_length; cursor += 1) {
        if (input_data[cursor] == '?') {
            query_start = cursor + 1;
            break;
        }
    }
    for (cursor = query_start; cursor < input_length;) {
        unsigned __int64 pair_end = cursor;
        unsigned __int64 equals = cursor;
        unsigned __int64 index;
        unsigned __int64 decoded = 0;
        int key_matches = 0;
        while (pair_end < input_length && input_data[pair_end] != '&' &&
               input_data[pair_end] != '#') pair_end += 1;
        if (pair_end == cursor) {
            if (pair_end < input_length && input_data[pair_end] == '#') break;
            return 0;
        }
        while (equals < pair_end && input_data[equals] != '=') equals += 1;
        if (equals == cursor || equals == pair_end) return 0;
        if (equals - cursor == key_length) {
            for (index = 0; index < key_length; index += 1) {
                if (input_data[cursor + index] != (unsigned char)key_data[index]) break;
            }
            key_matches = index == key_length;
        }
        if (key_matches) {
            if (matched) return 0;
            matched = 1;
            value_start = equals + 1;
            value_length = pair_end - value_start;
            for (index = 0; index < value_length; index += 1) {
                unsigned char value = input_data[value_start + index];
                if (value == '%') {
                    int high;
                    int low;
                    if (index + 2 >= value_length ||
                        (high = http_query_hex_value(input_data[value_start + index + 1])) < 0 ||
                        (low = http_query_hex_value(input_data[value_start + index + 2])) < 0) return 0;
                    (void)high;
                    (void)low;
                    index += 2;
                }
                if (decoded == (unsigned __int64)-1) return 0;
                decoded += 1;
            }
            decoded_length = decoded;
        }
        if (pair_end == input_length || input_data[pair_end] == '#') break;
        cursor = pair_end + 1;
    }
    if (!matched || decoded_length == 0 || output_length < decoded_length) return 0;
    result_length = decoded_length;
    for (cursor = 0; cursor < value_length; cursor += 1) {
        unsigned char value = input_data[value_start + cursor];
        if (value == '+') {
            output_data[output_offset] = ' ';
        } else if (value == '%') {
            int high = http_query_hex_value(input_data[value_start + cursor + 1]);
            int low = http_query_hex_value(input_data[value_start + cursor + 2]);
            output_data[output_offset] = (unsigned char)(high * 16 + low);
        } else {
            output_data[output_offset] = value;
        }
        output_offset += 1;
        if (value == '%') cursor += 2;
    }
    return result_length;
}

int http_query_param_exact(const unsigned char *input_data,
                           unsigned __int64 input_length,
                           const char *key_data,
                           unsigned __int64 key_length,
                           unsigned char *output_data,
                           unsigned __int64 output_length,
                           unsigned __int64 *output_size,
                           unsigned __int64 output_size_capacity) {
    unsigned __int64 query_start = 0;
    unsigned __int64 cursor;
    unsigned __int64 value_start = 0;
    unsigned __int64 value_length = 0;
    unsigned __int64 decoded_length = 0;
    unsigned __int64 output_offset = 0;
    int matched = 0;
    if (input_data == 0 || key_data == 0 || key_length == 0 || output_data == 0 ||
        output_size == 0 || output_size_capacity == 0) return 0;
    for (cursor = 0; cursor < input_length; cursor += 1) {
        if (input_data[cursor] == '?') {
            query_start = cursor + 1;
            break;
        }
    }
    for (cursor = query_start; cursor < input_length;) {
        unsigned __int64 pair_end = cursor;
        unsigned __int64 equals = cursor;
        unsigned __int64 index;
        unsigned __int64 decoded = 0;
        int key_matches = 0;
        while (pair_end < input_length && input_data[pair_end] != '&' &&
               input_data[pair_end] != '#') pair_end += 1;
        if (pair_end == cursor) {
            if (pair_end < input_length && input_data[pair_end] == '#') break;
            return 0;
        }
        while (equals < pair_end && input_data[equals] != '=') equals += 1;
        if (equals == cursor || equals == pair_end) return 0;
        if (equals - cursor == key_length) {
            for (index = 0; index < key_length; index += 1) {
                if (input_data[cursor + index] != (unsigned char)key_data[index]) break;
            }
            key_matches = index == key_length;
        }
        if (key_matches) {
            if (matched) return 0;
            matched = 1;
            value_start = equals + 1;
            value_length = pair_end - value_start;
            for (index = 0; index < value_length; index += 1) {
                unsigned char value = input_data[value_start + index];
                if (value == '%') {
                    int high;
                    int low;
                    if (index + 2 >= value_length ||
                        (high = http_query_hex_value(input_data[value_start + index + 1])) < 0 ||
                        (low = http_query_hex_value(input_data[value_start + index + 2])) < 0) return 0;
                    (void)high;
                    (void)low;
                    index += 2;
                }
                if (decoded == (unsigned __int64)-1) return 0;
                decoded += 1;
            }
            decoded_length = decoded;
        }
        if (pair_end == input_length || input_data[pair_end] == '#') break;
        cursor = pair_end + 1;
    }
    if (!matched || output_length < decoded_length) return 0;
    for (cursor = 0; cursor < value_length; cursor += 1) {
        unsigned char value = input_data[value_start + cursor];
        if (value == '+') {
            output_data[output_offset] = ' ';
        } else if (value == '%') {
            int high = http_query_hex_value(input_data[value_start + cursor + 1]);
            int low = http_query_hex_value(input_data[value_start + cursor + 2]);
            output_data[output_offset] = (unsigned char)(high * 16 + low);
        } else {
            output_data[output_offset] = value;
        }
        output_offset += 1;
        if (value == '%') cursor += 2;
    }
    *output_size = decoded_length;
    return 1;
}

/* Read one application/x-www-form-urlencoded field from a bounded request
 * body prefix. The body framing is validated before the field is decoded and
 * caller-owned output is published only after the complete pair list passes. */
static int http_form_param_exact_range(const unsigned char *input_data,
                                       unsigned __int64 body_start,
                                       unsigned __int64 body_length,
                                       const char *key_data,
                                       unsigned __int64 key_length,
                                       unsigned char *output_data,
                                       unsigned __int64 output_length,
                                       unsigned __int64 *output_size,
                                       unsigned __int64 output_size_capacity) {
    unsigned __int64 body_end;
    unsigned __int64 cursor;
    unsigned __int64 value_start = 0;
    unsigned __int64 value_length = 0;
    unsigned __int64 decoded_length = 0;
    unsigned __int64 output_offset = 0;
    int matched = 0;
    if (input_data == 0 || key_data == 0 || key_length == 0 || output_data == 0 ||
        output_size == 0 || output_size_capacity == 0 ||
        body_length > (unsigned __int64)-1 - body_start) return 0;
    body_end = body_start + body_length;
    for (cursor = body_start; cursor < body_end;) {
        unsigned __int64 pair_end = cursor;
        unsigned __int64 equals = cursor;
        unsigned __int64 index;
        unsigned __int64 decoded = 0;
        int key_matches = 0;
        while (pair_end < body_end && input_data[pair_end] != '&') pair_end += 1;
        if (pair_end == cursor) return 0;
        while (equals < pair_end && input_data[equals] != '=') equals += 1;
        if (equals == cursor || equals == pair_end) return 0;
        if (equals - cursor == key_length) {
            for (index = 0; index < key_length; index += 1) {
                if (input_data[cursor + index] != (unsigned char)key_data[index]) break;
            }
            key_matches = index == key_length;
        }
        if (key_matches) {
            if (matched) return 0;
            matched = 1;
            value_start = equals + 1;
            value_length = pair_end - value_start;
            for (index = 0; index < value_length; index += 1) {
                unsigned char value = input_data[value_start + index];
                if (value == '%') {
                    int high;
                    int low;
                    if (index + 2 >= value_length ||
                        (high = http_query_hex_value(input_data[value_start + index + 1])) < 0 ||
                        (low = http_query_hex_value(input_data[value_start + index + 2])) < 0) return 0;
                    (void)high;
                    (void)low;
                    index += 2;
                }
                if (decoded == (unsigned __int64)-1) return 0;
                decoded += 1;
            }
            decoded_length = decoded;
        }
        if (pair_end == body_end) break;
        cursor = pair_end + 1;
    }
    if (!matched || output_length < decoded_length) return 0;
    for (cursor = 0; cursor < value_length; cursor += 1) {
        unsigned char value = input_data[value_start + cursor];
        if (value == '+') {
            output_data[output_offset] = ' ';
        } else if (value == '%') {
            int high = http_query_hex_value(input_data[value_start + cursor + 1]);
            int low = http_query_hex_value(input_data[value_start + cursor + 2]);
            output_data[output_offset] = (unsigned char)(high * 16 + low);
        } else {
            output_data[output_offset] = value;
        }
        output_offset += 1;
        if (value == '%') cursor += 2;
    }
    *output_size = decoded_length;
    return 1;
}

int http_form_param_exact_prefix(const unsigned char *input_data,
                                 unsigned __int64 input_capacity,
                                 unsigned __int64 input_length,
                                 const char *key_data,
                                 unsigned __int64 key_length,
                                 unsigned char *output_data,
                                 unsigned __int64 output_length,
                                 unsigned __int64 *output_size,
                                 unsigned __int64 output_size_capacity) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    unsigned __int64 content_type_start;
    unsigned __int64 content_type_length;
    int has_content_length;
    int has_transfer_encoding;
    static const char expected_content_type[] = "application/x-www-form-urlencoded";
    if (input_length > input_capacity ||
        !http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start,
                              &body_start, &content_length,
                              &has_content_length, &has_transfer_encoding) ||
        !has_content_length || has_transfer_encoding || body_start > input_length ||
        content_length > input_length - body_start) return 0;
    if (http_request_find_header(input_data, input_length, headers_start,
                                 "Content-Type", 12,
                                 &content_type_start, &content_type_length) != 1 ||
        content_type_length != sizeof(expected_content_type) - 1) return 0;
    {
        unsigned __int64 content_type_index;
        for (content_type_index = 0;
             content_type_index < content_type_length;
             content_type_index += 1) {
            unsigned char value = input_data[content_type_start + content_type_index];
            unsigned char expected = (unsigned char)expected_content_type[content_type_index];
            if (value >= 'A' && value <= 'Z') value = (unsigned char)(value + ('a' - 'A'));
            if (value != expected) return 0;
        }
    }
    return http_form_param_exact_range(
        input_data, body_start, content_length, key_data, key_length,
        output_data, output_length, output_size, output_size_capacity);
}

static int http_multipart_ci_equal(const unsigned char *data,
                                   unsigned __int64 start,
                                   unsigned __int64 length,
                                   const char *expected,
                                   unsigned __int64 expected_length) {
    unsigned __int64 index;
    if (data == 0 || expected == 0 || length != expected_length) return 0;
    for (index = 0; index < length; index += 1) {
        unsigned char value = data[start + index];
        unsigned char wanted = (unsigned char)expected[index];
        if (value >= 'A' && value <= 'Z') value = (unsigned char)(value + 32);
        if (wanted >= 'A' && wanted <= 'Z') wanted = (unsigned char)(wanted + 32);
        if (value != wanted) return 0;
    }
    return 1;
}

static int http_multipart_bytes_equal(const unsigned char *data,
                                      unsigned __int64 start,
                                      unsigned __int64 length,
                                      const char *expected,
                                      unsigned __int64 expected_length) {
    unsigned __int64 index;
    if (data == 0 || expected == 0 || length != expected_length) return 0;
    for (index = 0; index < length; index += 1) {
        if (data[start + index] != (unsigned char)expected[index]) return 0;
    }
    return 1;
}

static int http_multipart_boundary(const unsigned char *data,
                                   unsigned __int64 start,
                                   unsigned __int64 length,
                                   unsigned __int64 *boundary_start,
                                   unsigned __int64 *boundary_length) {
    static const char prefix[] = "multipart/form-data; boundary=";
    unsigned __int64 prefix_length = sizeof(prefix) - 1;
    unsigned __int64 index;
    if (data == 0 || boundary_start == 0 || boundary_length == 0 ||
        length <= prefix_length ||
        !http_multipart_ci_equal(data, start, prefix_length, prefix, prefix_length)) {
        return 0;
    }
    *boundary_start = start + prefix_length;
    *boundary_length = length - prefix_length;
    if (*boundary_length == 0 || *boundary_length > 70) return 0;
    for (index = 0; index < *boundary_length; index += 1) {
        unsigned char value = data[*boundary_start + index];
        if (value <= 32 || value >= 127 || value == '"' || value == ';') return 0;
    }
    return 1;
}

static int http_multipart_disposition_matches(const unsigned char *data,
                                              unsigned __int64 start,
                                              unsigned __int64 length,
                                              const char *key_data,
                                              unsigned __int64 key_length) {
    unsigned __int64 cursor = start;
    unsigned __int64 end = start + length;
    unsigned __int64 token_start;
    unsigned __int64 token_length;
    unsigned __int64 value_start;
    unsigned __int64 value_length;
    int name_present = 0;
    int name_matches = 0;
    if (data == 0 || key_data == 0 || key_length == 0 || end < start) return 0;
    while (cursor < end && (data[cursor] == ' ' || data[cursor] == '\t')) cursor += 1;
    token_start = cursor;
    while (cursor < end && http_request_is_token(data[cursor])) cursor += 1;
    if (!http_multipart_ci_equal(data, token_start, cursor - token_start,
                                 "form-data", 9)) return 0;
    for (;;) {
        while (cursor < end && (data[cursor] == ' ' || data[cursor] == '\t')) cursor += 1;
        if (cursor == end) return name_present ? (name_matches ? 2 : 1) : 0;
        if (data[cursor] != ';') return 0;
        cursor += 1;
        while (cursor < end && (data[cursor] == ' ' || data[cursor] == '\t')) cursor += 1;
        token_start = cursor;
        while (cursor < end && http_request_is_token(data[cursor])) cursor += 1;
        token_length = cursor - token_start;
        while (cursor < end && (data[cursor] == ' ' || data[cursor] == '\t')) cursor += 1;
        if (token_length == 0 || cursor == end || data[cursor] != '=') return 0;
        cursor += 1;
        while (cursor < end && (data[cursor] == ' ' || data[cursor] == '\t')) cursor += 1;
        if (cursor == end || data[cursor] != '"') return 0;
        cursor += 1;
        value_start = cursor;
        while (cursor < end && data[cursor] != '"') {
            if (data[cursor] == '\r' || data[cursor] == '\n') return 0;
            cursor += 1;
        }
        if (cursor == end) return 0;
        value_length = cursor - value_start;
        cursor += 1;
        if (http_multipart_ci_equal(data, token_start, token_length, "name", 4)) {
            if (name_present) return 0;
            name_present = 1;
            name_matches = http_multipart_bytes_equal(data, value_start, value_length,
                                                      key_data, key_length);
        }
    }
}

int http_multipart_part_exact_prefix(const unsigned char *input_data,
                                     unsigned __int64 input_capacity,
                                     unsigned __int64 input_length,
                                     const char *key_data,
                                     unsigned __int64 key_length,
                                     unsigned char *output_data,
                                     unsigned __int64 output_length,
                                     unsigned __int64 *output_size,
                                     unsigned __int64 output_size_capacity) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    unsigned __int64 content_type_start;
    unsigned __int64 content_type_length;
    unsigned __int64 boundary_start;
    unsigned __int64 boundary_length;
    unsigned __int64 body_end;
    unsigned __int64 marker_length;
    unsigned __int64 cursor;
    unsigned __int64 header_end;
    unsigned __int64 search;
    unsigned __int64 delimiter_end;
    unsigned __int64 part_content_start;
    unsigned __int64 part_content_length;
    unsigned __int64 found_start = 0;
    unsigned __int64 found_length = 0;
    unsigned __int64 index;
    int has_content_length;
    int has_transfer_encoding;
    int disposition_found;
    int disposition_header_seen;
    int disposition_result;
    int field_found = 0;
    int final_boundary;
    static const char marker_prefix[] = "--";
    if (input_data == 0 || key_data == 0 || key_length == 0 || output_data == 0 ||
        output_size == 0 || output_size_capacity == 0 || input_length > input_capacity) return 0;
    if (!http_request_parse_line(input_data, input_length, &method_start, &method_length,
                                 &target_start, &target_length, &headers_start) ||
        !http_request_headers(input_data, input_length, headers_start, &body_start,
                              &content_length, &has_content_length, &has_transfer_encoding) ||
        !has_content_length || has_transfer_encoding || body_start > input_length ||
        content_length > input_length - body_start ||
        http_request_find_header(input_data, input_length, headers_start, "Content-Type", 12,
                                 &content_type_start, &content_type_length) != 1 ||
        !http_multipart_boundary(input_data, content_type_start, content_type_length,
                                 &boundary_start, &boundary_length) ||
        content_length > (unsigned __int64)-1 - body_start) return 0;
    body_end = body_start + content_length;
    marker_length = boundary_length + 2;
    if (body_start + marker_length + 2 > body_end ||
        input_data[body_start] != '-' || input_data[body_start + 1] != '-') return 0;
    for (index = 0; index < boundary_length; index += 1) {
        if (input_data[body_start + 2 + index] != input_data[boundary_start + index]) return 0;
    }
    cursor = body_start + marker_length;
    if (cursor + 2 > body_end || input_data[cursor] != '\r' || input_data[cursor + 1] != '\n') return 0;
    cursor += 2;
    for (;;) {
        if (cursor >= body_end) return 0;
        header_end = 0;
        for (search = cursor; search + 3 < body_end; search += 1) {
            if (input_data[search] == '\r' && input_data[search + 1] == '\n' &&
                input_data[search + 2] == '\r' && input_data[search + 3] == '\n') {
                header_end = search;
                break;
            }
        }
        if (header_end == 0 || header_end < cursor) return 0;
        disposition_found = 0;
        disposition_header_seen = 0;
        {
            unsigned __int64 header_cursor = cursor;
            unsigned __int64 next_header;
            unsigned __int64 name_start;
            unsigned __int64 name_length;
            unsigned __int64 value_start;
            unsigned __int64 value_length;
            int header_result;
            for (;;) {
                header_result = http_request_next_header(input_data, header_end + 4, header_cursor,
                                                         &next_header, &name_start, &name_length,
                                                         &value_start, &value_length);
                if (header_result < 0) return 0;
                if (header_result == 0) break;
                if (http_request_header_name_equals(input_data, name_start, name_length,
                                                    "Content-Disposition", 19)) {
                    if (disposition_header_seen) return 0;
                    disposition_header_seen = 1;
                    disposition_result = http_multipart_disposition_matches(
                        input_data, value_start, value_length, key_data, key_length);
                    if (disposition_result == 0) return 0;
                    disposition_found = disposition_result == 2;
                }
                header_cursor = next_header;
            }
        }
        if (!disposition_header_seen) return 0;
        part_content_start = header_end + 4;
        delimiter_end = 0;
        final_boundary = 0;
        for (search = part_content_start; search + marker_length + 4 <= body_end; search += 1) {
            if (input_data[search] != '\r' || input_data[search + 1] != '\n') continue;
            for (index = 0; index < marker_length; index += 1) {
                if (input_data[search + 2 + index] != (index < 2 ? (unsigned char)marker_prefix[index] : input_data[boundary_start + index - 2])) break;
            }
            if (index != marker_length) continue;
            delimiter_end = search + 2 + marker_length;
            if (delimiter_end + 2 <= body_end && input_data[delimiter_end] == '-' && input_data[delimiter_end + 1] == '-') {
                final_boundary = 1;
                if (delimiter_end + 2 != body_end &&
                    (delimiter_end + 4 != body_end || input_data[delimiter_end + 2] != '\r' || input_data[delimiter_end + 3] != '\n')) return 0;
                break;
            }
            if (delimiter_end + 2 <= body_end && input_data[delimiter_end] == '\r' && input_data[delimiter_end + 1] == '\n') break;
            delimiter_end = 0;
        }
        if (delimiter_end == 0) return 0;
        part_content_length = search - part_content_start;
        if (disposition_found) {
            if (field_found) return 0;
            field_found = 1;
            found_start = part_content_start;
            found_length = part_content_length;
        }
        if (final_boundary) break;
        cursor = delimiter_end + 2;
    }
    if (!field_found || output_length < found_length) return 0;
    for (index = 0; index < found_length; index += 1) output_data[index] = input_data[found_start + index];
    *output_size = found_length;
    return 1;
}

static int http_request_bytes_equal(const unsigned char *input_data,
                                    unsigned __int64 input_start,
                                    unsigned __int64 input_length,
                                    const char *expected_data,
                                    unsigned __int64 expected_length) {
    unsigned __int64 index;
    if (expected_data == 0 || input_length != expected_length) {
        return 0;
    }
    for (index = 0; index < input_length; index += 1) {
        if (input_data[input_start + index] !=
            (unsigned char)expected_data[index]) {
            return 0;
        }
    }
    return 1;
}

int http_route_match(const unsigned char *input_data,
                     unsigned __int64 input_length,
                     const char *expected_method_data,
                     unsigned __int64 expected_method_length,
                     const char *expected_target_data,
                     unsigned __int64 expected_target_length) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    if (!http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start) ||
        !http_request_bytes_equal(input_data, method_start, method_length,
                                  expected_method_data, expected_method_length) ||
        !http_request_bytes_equal(input_data, target_start, target_length,
                                  expected_target_data, expected_target_length)) {
        return 0;
    }
    return 1;
}

int http_route_match_prefix(const unsigned char *input_data,
                            unsigned __int64 input_capacity,
                            unsigned __int64 input_length,
                            const char *expected_method_data,
                            unsigned __int64 expected_method_length,
                            const char *expected_target_data,
                            unsigned __int64 expected_target_length) {
    if (input_length > input_capacity) return 0;
    return http_route_match(input_data, input_length, expected_method_data,
                            expected_method_length, expected_target_data,
                            expected_target_length);
}

#define JADREN_HTTP_ROUTE_CAPACITY 16
#define JADREN_HTTP_ROUTE_METHOD_CAPACITY 16
#define JADREN_HTTP_ROUTE_TARGET_CAPACITY 128
#define JADREN_HTTP_ROUTE_CONTENT_TYPE_CAPACITY 64
#define JADREN_HTTP_ROUTE_BODY_CAPACITY 2048
typedef struct JadrenHttpRoute {
    int active;
    int match_mode;
    unsigned short status;
    unsigned __int64 method_length;
    unsigned __int64 target_length;
    unsigned __int64 content_type_length;
    unsigned __int64 body_length;
    char method[JADREN_HTTP_ROUTE_METHOD_CAPACITY];
    char target[JADREN_HTTP_ROUTE_TARGET_CAPACITY];
    char content_type[JADREN_HTTP_ROUTE_CONTENT_TYPE_CAPACITY];
    unsigned char body[JADREN_HTTP_ROUTE_BODY_CAPACITY];
} JadrenHttpRoute;

static JadrenHttpRoute jadren_http_routes[JADREN_HTTP_ROUTE_CAPACITY];

static int jadren_http_route_copy(char *destination,
                                  unsigned __int64 capacity,
                                  const char *source,
                                  unsigned __int64 length) {
    unsigned __int64 index;
    if (source == 0 || length == 0 || length >= capacity) {
        return 0;
    }
    for (index = 0; index < length; index += 1) {
        if (source[index] == 0) {
            return 0;
        }
        destination[index] = source[index];
    }
    destination[index] = 0;
    return 1;
}

static int jadren_http_route_valid(const char *source,
                                   unsigned __int64 capacity,
                                   unsigned __int64 length) {
    unsigned __int64 index;
    if (source == 0 || length == 0 || length >= capacity) {
        return 0;
    }
    for (index = 0; index < length; index += 1) {
        if (source[index] == 0) {
            return 0;
        }
    }
    return 1;
}

static int jadren_http_route_matches(const JadrenHttpRoute *route,
                                     const unsigned char *input_data,
                                     unsigned __int64 method_start,
                                     unsigned __int64 method_length,
                                     unsigned __int64 target_start,
                                     unsigned __int64 target_length) {
    return route->method_length == method_length &&
           route->target_length == target_length &&
           http_request_bytes_equal(input_data, method_start, method_length,
                                    route->method, route->method_length) &&
           http_request_bytes_equal(input_data, target_start, target_length,
                                    route->target, route->target_length);
}

static int jadren_http_route_prefix_matches(
    const JadrenHttpRoute *route, const unsigned char *input_data,
    unsigned __int64 method_start, unsigned __int64 method_length,
    unsigned __int64 target_start, unsigned __int64 target_length) {
    return route->method_length == method_length &&
           route->target_length <= target_length &&
           http_request_bytes_equal(input_data, method_start, method_length,
                                    route->method, route->method_length) &&
           http_request_bytes_equal(input_data, target_start,
                                    route->target_length, route->target,
                                    route->target_length);
}

void http_router_clear(void) {
    int index;
    for (index = 0; index < JADREN_HTTP_ROUTE_CAPACITY; index += 1) {
        jadren_http_routes[index].active = 0;
    }
}

int http_router_remove(const char *method_data, unsigned __int64 method_length,
                       const char *target_data, unsigned __int64 target_length) {
    int index;
    if (!jadren_http_route_valid(method_data,
                                 JADREN_HTTP_ROUTE_METHOD_CAPACITY,
                                 method_length) ||
        !jadren_http_route_valid(target_data,
                                 JADREN_HTTP_ROUTE_TARGET_CAPACITY,
                                 target_length)) {
        return 0;
    }
    for (index = 0; index < JADREN_HTTP_ROUTE_CAPACITY; index += 1) {
        JadrenHttpRoute *route = &jadren_http_routes[index];
        if (route->active && route->match_mode == 0 &&
            route->method_length == method_length &&
            route->target_length == target_length &&
            http_request_bytes_equal((const unsigned char *)method_data, 0,
                                     method_length, route->method,
                                     route->method_length) &&
            http_request_bytes_equal((const unsigned char *)target_data, 0,
                                     target_length, route->target,
                                     route->target_length)) {
            route->active = 0;
            return 1;
        }
    }
    return 0;
}

unsigned __int64 http_router_count(void) {
    unsigned __int64 count = 0;
    int index;
    for (index = 0; index < JADREN_HTTP_ROUTE_CAPACITY; index += 1) {
        if (jadren_http_routes[index].active) {
            count += 1ULL;
        }
    }
    return count;
}

int http_router_add(const char *method_data, unsigned __int64 method_length,
                    const char *target_data, unsigned __int64 target_length,
                    unsigned short status, const char *content_type_data,
                    unsigned __int64 content_type_length,
                    const unsigned char *body_data,
                    unsigned __int64 body_length) {
    int index;
    int free_index = -1;
    if (!jadren_http_route_valid(method_data,
                                 JADREN_HTTP_ROUTE_METHOD_CAPACITY,
                                 method_length) ||
        !jadren_http_route_valid(target_data,
                                 JADREN_HTTP_ROUTE_TARGET_CAPACITY,
                                 target_length) ||
        !jadren_http_route_valid(content_type_data,
                                 JADREN_HTTP_ROUTE_CONTENT_TYPE_CAPACITY,
                                 content_type_length) ||
        body_length > JADREN_HTTP_ROUTE_BODY_CAPACITY ||
        (body_data == 0 && body_length > 0)) {
        return 0;
    }
    for (index = 0; index < JADREN_HTTP_ROUTE_CAPACITY; index += 1) {
        JadrenHttpRoute *route = &jadren_http_routes[index];
        if (!route->active) {
            if (free_index < 0) {
                free_index = index;
            }
            continue;
        }
        if (route->match_mode == 0 && route->method_length == method_length &&
            route->target_length == target_length &&
            http_request_bytes_equal((const unsigned char *)method_data, 0,
                                     method_length, route->method,
                                     route->method_length) &&
            http_request_bytes_equal((const unsigned char *)target_data, 0,
                                     target_length, route->target,
                                     route->target_length)) {
            free_index = index;
            break;
        }
    }
    if (free_index < 0 ||
        !jadren_http_route_copy(jadren_http_routes[free_index].method,
                                JADREN_HTTP_ROUTE_METHOD_CAPACITY,
                                method_data, method_length) ||
        !jadren_http_route_copy(jadren_http_routes[free_index].target,
                                JADREN_HTTP_ROUTE_TARGET_CAPACITY,
                                target_data, target_length) ||
        !jadren_http_route_copy(jadren_http_routes[free_index].content_type,
                                JADREN_HTTP_ROUTE_CONTENT_TYPE_CAPACITY,
                                content_type_data, content_type_length)) {
        return 0;
    }
    for (index = 0; (unsigned __int64)index < body_length; index += 1) {
        jadren_http_routes[free_index].body[index] = body_data[index];
    }
    jadren_http_routes[free_index].status = status;
    jadren_http_routes[free_index].match_mode = 0;
    jadren_http_routes[free_index].method_length = method_length;
    jadren_http_routes[free_index].target_length = target_length;
    jadren_http_routes[free_index].content_type_length = content_type_length;
    jadren_http_routes[free_index].body_length = body_length;
    jadren_http_routes[free_index].active = 1;
    return 1;
}

/* Register a route from an explicit caller-owned body prefix. The hidden
 * slice capacity is checked before the existing route copier is called. */
int http_router_add_exact(const char *method_data, unsigned __int64 method_length,
                          const char *target_data, unsigned __int64 target_length,
                          unsigned short status, const char *content_type_data,
                          unsigned __int64 content_type_length,
                          const unsigned char *body_data,
                          unsigned __int64 body_capacity,
                          unsigned __int64 body_length) {
    if (body_length > body_capacity) return 0;
    return http_router_add(method_data, method_length, target_data, target_length,
                           status, content_type_data, content_type_length,
                           body_data, body_length);
}

int http_router_add_prefix(const char *method_data,
                           unsigned __int64 method_length,
                           const char *target_data,
                           unsigned __int64 target_length,
                           unsigned short status,
                           const char *content_type_data,
                           unsigned __int64 content_type_length,
                           const unsigned char *body_data,
                           unsigned __int64 body_length) {
    int index;
    int free_index = -1;
    if (!jadren_http_route_valid(method_data,
                                 JADREN_HTTP_ROUTE_METHOD_CAPACITY,
                                 method_length) ||
        !jadren_http_route_valid(target_data,
                                 JADREN_HTTP_ROUTE_TARGET_CAPACITY,
                                 target_length) ||
        !jadren_http_route_valid(content_type_data,
                                 JADREN_HTTP_ROUTE_CONTENT_TYPE_CAPACITY,
                                 content_type_length) ||
        body_length > JADREN_HTTP_ROUTE_BODY_CAPACITY ||
        (body_data == 0 && body_length > 0)) {
        return 0;
    }
    for (index = 0; index < JADREN_HTTP_ROUTE_CAPACITY; index += 1) {
        JadrenHttpRoute *route = &jadren_http_routes[index];
        if (!route->active) {
            if (free_index < 0) free_index = index;
            continue;
        }
        if (route->match_mode == 1 && route->method_length == method_length &&
            route->target_length == target_length &&
            http_request_bytes_equal((const unsigned char *)method_data, 0,
                                     method_length, route->method,
                                     route->method_length) &&
            http_request_bytes_equal((const unsigned char *)target_data, 0,
                                     target_length, route->target,
                                     route->target_length)) {
            free_index = index;
            break;
        }
    }
    if (free_index < 0 ||
        !jadren_http_route_copy(jadren_http_routes[free_index].method,
                                JADREN_HTTP_ROUTE_METHOD_CAPACITY,
                                method_data, method_length) ||
        !jadren_http_route_copy(jadren_http_routes[free_index].target,
                                JADREN_HTTP_ROUTE_TARGET_CAPACITY,
                                target_data, target_length) ||
        !jadren_http_route_copy(jadren_http_routes[free_index].content_type,
                                JADREN_HTTP_ROUTE_CONTENT_TYPE_CAPACITY,
                                content_type_data, content_type_length)) {
        return 0;
    }
    for (index = 0; (unsigned __int64)index < body_length; index += 1) {
        jadren_http_routes[free_index].body[index] = body_data[index];
    }
    jadren_http_routes[free_index].status = status;
    jadren_http_routes[free_index].match_mode = 1;
    jadren_http_routes[free_index].method_length = method_length;
    jadren_http_routes[free_index].target_length = target_length;
    jadren_http_routes[free_index].content_type_length = content_type_length;
    jadren_http_routes[free_index].body_length = body_length;
    jadren_http_routes[free_index].active = 1;
    return 1;
}

int http_router_remove_prefix(const char *method_data,
                              unsigned __int64 method_length,
                              const char *target_data,
                              unsigned __int64 target_length) {
    int index;
    if (!jadren_http_route_valid(method_data,
                                 JADREN_HTTP_ROUTE_METHOD_CAPACITY,
                                 method_length) ||
        !jadren_http_route_valid(target_data,
                                 JADREN_HTTP_ROUTE_TARGET_CAPACITY,
                                 target_length)) {
        return 0;
    }
    for (index = 0; index < JADREN_HTTP_ROUTE_CAPACITY; index += 1) {
        JadrenHttpRoute *route = &jadren_http_routes[index];
        if (route->active && route->match_mode == 1 &&
            route->method_length == method_length &&
            route->target_length == target_length &&
            http_request_bytes_equal((const unsigned char *)method_data, 0,
                                     method_length, route->method,
                                     route->method_length) &&
            http_request_bytes_equal((const unsigned char *)target_data, 0,
                                     target_length, route->target,
                                     route->target_length)) {
            route->active = 0;
            return 1;
        }
    }
    return 0;
}

static unsigned __int64 jadren_http_router_respond_mode(
    const unsigned char *input_data, unsigned __int64 input_length,
    unsigned char *output_data, unsigned __int64 output_length, int keep_alive) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    int index;
    int best_prefix = -1;
    unsigned __int64 best_prefix_length = 0;
    if (!http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start)) {
        return 0;
    }
    for (index = 0; index < JADREN_HTTP_ROUTE_CAPACITY; index += 1) {
        JadrenHttpRoute *route = &jadren_http_routes[index];
        if (route->active && route->match_mode == 0 &&
            jadren_http_route_matches(route, input_data, method_start,
                                      method_length, target_start, target_length)) {
            return http_response_write_mode(
                route->status, route->content_type, route->content_type_length,
                route->body, route->body_length, keep_alive, output_data,
                output_length);
        }
        if (route->active && route->match_mode == 1 &&
            route->target_length > best_prefix_length &&
            jadren_http_route_prefix_matches(route, input_data, method_start,
                                             method_length, target_start,
                                             target_length)) {
            best_prefix = index;
            best_prefix_length = route->target_length;
        }
    }
    if (best_prefix >= 0) {
        JadrenHttpRoute *route = &jadren_http_routes[best_prefix];
        unsigned __int64 response_length = http_response_write_mode(
            route->status, route->content_type, route->content_type_length,
            route->body, route->body_length, keep_alive, output_data,
            output_length);
        return response_length;
    }
    return http_response_write_mode(404, "text/plain", 10,
                                    (const unsigned char *)"not found", 9,
                                    keep_alive, output_data, output_length);
}

unsigned __int64 http_router_respond(const unsigned char *input_data,
                                     unsigned __int64 input_length,
                                     unsigned char *output_data,
                                     unsigned __int64 output_length) {
    return jadren_http_router_respond_mode(input_data, input_length, output_data,
                                           output_length, 0);
}

/* Dispatch only the explicit valid request prefix from a larger caller-owned
 * receive buffer. The output remains bounded by its hidden slice capacity. */
unsigned __int64 http_router_respond_prefix(
    const unsigned char *input_data, unsigned __int64 input_capacity,
    unsigned __int64 input_length, unsigned char *output_data,
    unsigned __int64 output_length) {
    if (input_length > input_capacity) return 0;
    return jadren_http_router_respond_mode(input_data, input_length, output_data,
                                           output_length, 0);
}

/* Dispatch a bounded request through the registered routes and serialize a
 * one-shot HTTP/1.1 chunked response. Chunked responses close the connection;
 * callers that need keep-alive must continue using the session responder. */
static unsigned __int64 jadren_http_router_respond_chunked_mode(
    const unsigned char *input_data, unsigned __int64 input_length,
    unsigned char *output_data, unsigned __int64 output_length) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    int index;
    int best_prefix = -1;
    unsigned __int64 best_prefix_length = 0;
    if (!http_request_parse_line(input_data, input_length, &method_start,
                                 &method_length, &target_start, &target_length,
                                 &headers_start)) {
        return 0;
    }
    for (index = 0; index < JADREN_HTTP_ROUTE_CAPACITY; index += 1) {
        JadrenHttpRoute *route = &jadren_http_routes[index];
        if (route->active && route->match_mode == 0 &&
            jadren_http_route_matches(route, input_data, method_start,
                                      method_length, target_start, target_length)) {
            return http_response_write_chunked_prefix_mode(
                route->status, route->content_type, route->content_type_length,
                route->body, sizeof(route->body), route->body_length,
                output_data, output_length);
        }
        if (route->active && route->match_mode == 1 &&
            route->target_length > best_prefix_length &&
            jadren_http_route_prefix_matches(route, input_data, method_start,
                                             method_length, target_start,
                                             target_length)) {
            best_prefix = index;
            best_prefix_length = route->target_length;
        }
    }
    if (best_prefix >= 0) {
        JadrenHttpRoute *route = &jadren_http_routes[best_prefix];
        return http_response_write_chunked_prefix_mode(
            route->status, route->content_type, route->content_type_length,
            route->body, sizeof(route->body), route->body_length, output_data,
            output_length);
    }
    return http_response_write_chunked_prefix_mode(
        404, "text/plain", 10, (const unsigned char *)"not found", 9, 9,
        output_data, output_length);
}

unsigned __int64 http_router_respond_chunked(
    const unsigned char *input_data, unsigned __int64 input_length,
    unsigned char *output_data, unsigned __int64 output_length) {
    return jadren_http_router_respond_chunked_mode(input_data, input_length,
                                                   output_data, output_length);
}

/* Dispatch only the explicit valid request prefix from a larger caller-owned
 * receive buffer and serialize the response with chunked framing. */
unsigned __int64 http_router_respond_chunked_prefix(
    const unsigned char *input_data, unsigned __int64 input_capacity,
    unsigned __int64 input_length, unsigned char *output_data,
    unsigned __int64 output_length) {
    if (input_length > input_capacity) return 0;
    return jadren_http_router_respond_chunked_mode(
        input_data, input_length, output_data, output_length);
}

#if JADREN_FILE_RUNTIME_HAS_NETWORK_SUPPORT
#define JADREN_HTTP_SESSION_CAPACITY 4
#define JADREN_HTTP_SESSION_CONNECTION_CAPACITY 8
#define JADREN_HTTP_SESSION_HEADER_LIMIT 8192
#define JADREN_HTTP_SESSION_BODY_LIMIT 8192
#define JADREN_HTTP_SESSION_REQUEST_CAPACITY 16384
#define JADREN_HTTP_SESSION_RESPONSE_CAPACITY 16384
#define JADREN_HTTP_SESSION_TLS_PATH_CAPACITY 512

#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
unsigned __int64 net_tls_open_server(unsigned __int64 socket_token,
                                     const char *certificate_data,
                                     unsigned __int64 certificate_length,
                                     const char *private_key_data,
                                     unsigned __int64 private_key_length);
unsigned int net_tls_step(unsigned __int64 token, unsigned int timeout_ms);
unsigned int net_tls_state(unsigned __int64 token);
unsigned __int64 net_tls_send(unsigned __int64 token,
                               const unsigned char *input_data,
                               unsigned __int64 input_length);
unsigned __int64 net_tls_receive(unsigned __int64 token,
                                  unsigned char *output_data,
                                  unsigned __int64 output_length);
int net_tls_close(unsigned __int64 token);
#endif

typedef struct JadrenHttpSessionConnection {
    int active;
    unsigned __int64 socket_token;
    unsigned __int64 tls_token;
    unsigned __int64 request_length;
    unsigned char request[JADREN_HTTP_SESSION_REQUEST_CAPACITY];
} JadrenHttpSessionConnection;

typedef struct JadrenHttpSession {
    int active;
    unsigned __int64 listener;
    unsigned int max_connections;
    unsigned int max_header_bytes;
    unsigned int max_body_bytes;
    unsigned int cursor;
    int tls_enabled;
    int chunked_mode;
    unsigned __int64 certificate_length;
    unsigned __int64 private_key_length;
    char certificate[JADREN_HTTP_SESSION_TLS_PATH_CAPACITY];
    char private_key[JADREN_HTTP_SESSION_TLS_PATH_CAPACITY];
    JadrenHttpSessionConnection connections[JADREN_HTTP_SESSION_CONNECTION_CAPACITY];
} JadrenHttpSession;

static JadrenHttpSession jadren_http_sessions[JADREN_HTTP_SESSION_CAPACITY];

static int jadren_http_session_headers_terminated(const unsigned char *data,
                                                  unsigned __int64 length) {
    unsigned __int64 index;
    if (data == 0) {
        return 0;
    }
    for (index = 3; index < length; index += 1) {
        if (data[index - 3] == '\r' && data[index - 2] == '\n' &&
            data[index - 1] == '\r' && data[index] == '\n') {
            return 1;
        }
    }
    return 0;
}

static int jadren_http_session_receive(JadrenHttpSessionConnection *connection,
                                        unsigned char *output_data,
                                        unsigned __int64 output_length) {
    int received;
    int error;
    if (connection == 0 || output_data == 0 || output_length == 0) {
        return 0;
    }
#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
    if (connection->tls_token != 0) {
        return (int)net_tls_receive(connection->tls_token, output_data, output_length);
    }
#endif
    if (connection->socket_token == 0) return 0;
    received = recv((JadrenSocket)(connection->socket_token - 1ULL), (char *)output_data,
                    (int)output_length, 0);
    if (received > 0) {
        return received;
    }
    if (received == 0) {
        return 0;
    }
    error = WSAGetLastError();
    return error == 10004 || error == 10035 || error == 10060 ? -1 : 0;
}

static int jadren_http_session_send_all(JadrenHttpSessionConnection *connection,
                                         const unsigned char *data,
                                         unsigned __int64 length) {
    unsigned __int64 offset = 0;
    if (connection == 0 || connection->socket_token == 0 ||
        (data == 0 && length > 0)) {
        return 0;
    }
    while (offset < length) {
#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
        if (connection->tls_token != 0) {
            unsigned __int64 sent = net_tls_send(connection->tls_token,
                                                 data + offset, length - offset);
            if (sent == 0 || sent > length - offset) return 0;
            offset += sent;
            continue;
        }
#endif
        unsigned __int64 sent = net_tcp_send_prefix(
            connection->socket_token, data + offset, length - offset,
            length - offset);
        if (sent == 0 || sent > length - offset) {
            return 0;
        }
        offset += sent;
    }
    return 1;
}

/* Reads one complete request frame into the session's bounded connection
 * buffer. The caller decides when to publish the frame and which response
 * bytes to send; this helper never routes, writes or closes the connection. */
static int jadren_http_session_read_frame(
    JadrenHttpSession *session, JadrenHttpSessionConnection *connection,
    unsigned int timeout_ms, unsigned __int64 *frame_length_output) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    unsigned __int64 frame_length;
    unsigned __int64 chunked_body_length;
    unsigned __int64 chunked_body_end;
    int has_content_length;
    int has_transfer_encoding;
    int receive_result;
    if (session == 0 || connection == 0 || !connection->active ||
        frame_length_output == 0) {
        return -1;
    }
#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
    if (session->tls_enabled) {
        unsigned int tls_state = net_tls_state(connection->tls_token);
        unsigned int tls_steps = 0U;
        while (tls_state == 1U && tls_steps < 8U) {
            (void)net_tls_step(connection->tls_token, timeout_ms);
            tls_state = net_tls_state(connection->tls_token);
            tls_steps += 1U;
        }
        if (tls_state == 4U || tls_state == 3U || tls_state == 0U) return -1;
        if (tls_state != 2U) return 0;
    }
#endif
    (void)net_socket_set_timeout(connection->socket_token,
                                 timeout_ms == 0 ? 1 : timeout_ms);
    for (;;) {
        frame_length = http_request_frame_length_prefix(
            connection->request, JADREN_HTTP_SESSION_REQUEST_CAPACITY,
            connection->request_length);
        if (frame_length != 0) break;
        if (http_request_parse_line(connection->request, connection->request_length,
                                    &method_start, &method_length, &target_start,
                                    &target_length, &headers_start) &&
            http_request_headers(connection->request, connection->request_length,
                                 headers_start, &body_start, &content_length,
                                 &has_content_length, &has_transfer_encoding)) {
            if (has_transfer_encoding && has_content_length) return -1;
            if (body_start > session->max_header_bytes ||
                (!has_transfer_encoding && has_content_length &&
                 content_length > session->max_body_bytes)) {
                return -1;
            }
            if (has_transfer_encoding) {
                frame_length = http_request_chunked_frame_length_prefix(
                    connection->request, JADREN_HTTP_SESSION_REQUEST_CAPACITY,
                    connection->request_length);
                if (frame_length != 0) break;
            } else if (!has_content_length) {
                frame_length = body_start;
                break;
            }
        } else if (jadren_http_session_headers_terminated(
                       connection->request, connection->request_length) ||
                   connection->request_length >= session->max_header_bytes) {
            return -1;
        }
        if (connection->request_length >= JADREN_HTTP_SESSION_REQUEST_CAPACITY) {
            return -1;
        }
        receive_result = jadren_http_session_receive(
            connection,
            connection->request + connection->request_length,
            JADREN_HTTP_SESSION_REQUEST_CAPACITY - connection->request_length);
        if (receive_result < 0) return 0;
        if (receive_result == 0) return -1;
        connection->request_length += (unsigned __int64)receive_result;
    }
    if (!http_request_parse_line(connection->request, connection->request_length,
                                 &method_start, &method_length, &target_start,
                                 &target_length, &headers_start) ||
        !http_request_headers(connection->request, connection->request_length,
                              headers_start, &body_start, &content_length,
                              &has_content_length, &has_transfer_encoding)) {
        return -1;
    }
    if (has_transfer_encoding && has_content_length) return -1;
    if (body_start > session->max_header_bytes ||
        (!has_transfer_encoding && has_content_length &&
         content_length > session->max_body_bytes)) {
        return -1;
    }
    if (has_transfer_encoding) {
        frame_length = http_request_chunked_frame_length_prefix(
            connection->request, JADREN_HTTP_SESSION_REQUEST_CAPACITY,
            connection->request_length);
        if (frame_length == 0 ||
            !http_response_chunked_scan(
                connection->request, frame_length, body_start, 0, 0,
                &chunked_body_length, &chunked_body_end) ||
            chunked_body_end != frame_length ||
            chunked_body_length > session->max_body_bytes) {
            return -1;
        }
    } else {
        frame_length = http_request_frame_length_prefix(
            connection->request, JADREN_HTTP_SESSION_REQUEST_CAPACITY,
            connection->request_length);
    }
    if (frame_length == 0 ||
        (!has_transfer_encoding &&
         (body_start > frame_length ||
          (has_content_length && frame_length - body_start != content_length) ||
          (!has_content_length && frame_length != body_start)))) {
        return -1;
    }
    *frame_length_output = frame_length;
    return 1;
}

static int jadren_http_session_wait_listener(unsigned __int64 listener,
                                             unsigned int timeout_ms) {
    JadrenFdSet read_set;
    JadrenTimeval timeout;
    if (listener == 0) {
        return 0;
    }
    read_set.count = 1;
    read_set.sockets[0] = (JadrenSocket)(listener - 1ULL);
    timeout.seconds = (long)(timeout_ms / 1000U);
    timeout.microseconds = (long)((timeout_ms % 1000U) * 1000U);
    return select(0, &read_set, 0, 0, &timeout) > 0;
}

static JadrenHttpSession *jadren_http_session_find(unsigned __int64 token) {
    if (token == 0 || token > JADREN_HTTP_SESSION_CAPACITY) {
        return 0;
    }
    return jadren_http_sessions + (token - 1ULL);
}

static unsigned __int64 jadren_http_session_connection_token(
    const JadrenHttpSession *session, unsigned int connection_index) {
    unsigned __int64 session_index;
    if (session == 0 || connection_index >= JADREN_HTTP_SESSION_CONNECTION_CAPACITY) {
        return 0;
    }
    session_index = (unsigned __int64)(session - jadren_http_sessions);
    return session_index * JADREN_HTTP_SESSION_CONNECTION_CAPACITY +
           (unsigned __int64)connection_index + 1ULL;
}

static JadrenHttpSessionConnection *jadren_http_session_find_connection(
    unsigned __int64 token, JadrenHttpSession **session_output) {
    unsigned __int64 raw;
    unsigned int session_index;
    unsigned int connection_index;
    JadrenHttpSession *session;
    if (token == 0) {
        return 0;
    }
    raw = token - 1ULL;
    session_index = (unsigned int)(raw / JADREN_HTTP_SESSION_CONNECTION_CAPACITY);
    connection_index = (unsigned int)(raw % JADREN_HTTP_SESSION_CONNECTION_CAPACITY);
    if (session_index >= JADREN_HTTP_SESSION_CAPACITY ||
        connection_index >= JADREN_HTTP_SESSION_CONNECTION_CAPACITY) {
        return 0;
    }
    session = &jadren_http_sessions[session_index];
    if (!session->active || connection_index >= session->max_connections ||
        !session->connections[connection_index].active) {
        return 0;
    }
    if (session_output != 0) {
        *session_output = session;
    }
    return &session->connections[connection_index];
}

static void jadren_http_session_close_connection(
    JadrenHttpSessionConnection *connection) {
    if (connection != 0 && connection->active) {
#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
        if (connection->tls_token != 0) {
            (void)net_tls_close(connection->tls_token);
        } else
#endif
        {
            (void)net_socket_close(connection->socket_token);
        }
        connection->active = 0;
        connection->socket_token = 0;
        connection->tls_token = 0;
        connection->request_length = 0;
    }
}

static int jadren_http_session_process_connection(
    JadrenHttpSession *session, JadrenHttpSessionConnection *connection,
    unsigned int timeout_ms) {
    unsigned __int64 method_start;
    unsigned __int64 method_length;
    unsigned __int64 target_start;
    unsigned __int64 target_length;
    unsigned __int64 headers_start;
    unsigned __int64 body_start;
    unsigned __int64 content_length;
    unsigned char response[JADREN_HTTP_SESSION_RESPONSE_CAPACITY];
    unsigned __int64 received;
    unsigned __int64 response_length;
    unsigned __int64 frame_length;
    unsigned __int64 chunked_body_length;
    unsigned __int64 chunked_body_end;
    int has_content_length;
    int has_transfer_encoding;
    int receive_result;
    int keep_alive;
    if (session == 0 || connection == 0 || !connection->active) {
        return -1;
    }
#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
    if (session->tls_enabled) {
        unsigned int tls_state = net_tls_state(connection->tls_token);
        unsigned int tls_steps = 0U;
        while (tls_state == 1U && tls_steps < 8U) {
            (void)net_tls_step(connection->tls_token, timeout_ms);
            tls_state = net_tls_state(connection->tls_token);
            tls_steps += 1U;
        }
        if (tls_state == 4U || tls_state == 3U || tls_state == 0U) return -1;
        if (tls_state != 2U) return 0;
    }
#endif
    (void)net_socket_set_timeout(connection->socket_token,
                                 timeout_ms == 0 ? 1 : timeout_ms);
    for (;;) {
        frame_length = http_request_frame_length_prefix(
            connection->request, JADREN_HTTP_SESSION_REQUEST_CAPACITY,
            connection->request_length);
        if (frame_length != 0) {
            break;
        }
        if (http_request_parse_line(connection->request, connection->request_length,
                                    &method_start, &method_length, &target_start,
                                    &target_length, &headers_start) &&
            http_request_headers(connection->request, connection->request_length,
                                 headers_start, &body_start, &content_length,
                                 &has_content_length, &has_transfer_encoding)) {
            if (has_transfer_encoding && has_content_length) {
                return -1;
            }
            if (body_start > session->max_header_bytes ||
                (!has_transfer_encoding && has_content_length &&
                 content_length > session->max_body_bytes)) {
                return -1;
            }
            if (has_transfer_encoding) {
                frame_length = http_request_chunked_frame_length_prefix(
                    connection->request, JADREN_HTTP_SESSION_REQUEST_CAPACITY,
                    connection->request_length);
                if (frame_length != 0) {
                    break;
                }
            } else if (!has_content_length) {
                frame_length = body_start;
                break;
            }
        } else if (jadren_http_session_headers_terminated(
                       connection->request, connection->request_length) ||
                   connection->request_length >= session->max_header_bytes) {
            return -1;
        }
        if (connection->request_length >= JADREN_HTTP_SESSION_REQUEST_CAPACITY) {
            return -1;
        }
        receive_result = jadren_http_session_receive(
            connection,
            connection->request + connection->request_length,
            JADREN_HTTP_SESSION_REQUEST_CAPACITY - connection->request_length);
        if (receive_result < 0) {
            return 0;
        }
        if (receive_result == 0) {
#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
            if (session->tls_enabled && net_tls_state(connection->tls_token) == 4U) {
                return -1;
            }
#endif
            return -1;
        }
        received = (unsigned __int64)receive_result;
        connection->request_length += received;
    }
    if (!http_request_parse_line(connection->request, connection->request_length,
                                 &method_start, &method_length, &target_start,
                                 &target_length, &headers_start)) {
        return jadren_http_session_headers_terminated(
                   connection->request, connection->request_length) ||
                   connection->request_length >= session->max_header_bytes
            ? -1
            : 0;
    }
    if (!http_request_headers(connection->request, connection->request_length,
                              headers_start, &body_start, &content_length,
                              &has_content_length, &has_transfer_encoding)) {
        return jadren_http_session_headers_terminated(
                   connection->request, connection->request_length) ||
                   connection->request_length >= session->max_header_bytes
            ? -1
            : 0;
    }
    if (has_transfer_encoding && has_content_length) {
        return -1;
    }
    if (body_start > session->max_header_bytes ||
        (!has_transfer_encoding && has_content_length &&
         content_length > session->max_body_bytes)) {
        return -1;
    }
    if (has_transfer_encoding) {
        frame_length = http_request_chunked_frame_length_prefix(
            connection->request, JADREN_HTTP_SESSION_REQUEST_CAPACITY,
            connection->request_length);
        if (frame_length == 0) {
            return connection->request_length >= JADREN_HTTP_SESSION_REQUEST_CAPACITY
                ? -1
                : 0;
        }
        if (!http_response_chunked_scan(
                connection->request, frame_length, body_start, 0, 0,
                &chunked_body_length, &chunked_body_end) ||
            chunked_body_end != frame_length ||
            chunked_body_length > session->max_body_bytes) {
            return -1;
        }
    } else {
        frame_length = http_request_frame_length_prefix(
            connection->request, JADREN_HTTP_SESSION_REQUEST_CAPACITY,
            connection->request_length);
    }
    if (frame_length == 0) {
        return 0;
    }
    if (!has_transfer_encoding &&
        (body_start > frame_length ||
         (has_content_length && frame_length - body_start != content_length) ||
         (!has_content_length && frame_length != body_start))) {
        return -1;
    }
    keep_alive = http_request_keep_alive(connection->request,
                                         frame_length);
    if (session->chunked_mode) {
        response_length = jadren_http_router_respond_chunked_mode(
            connection->request, frame_length, response, sizeof(response));
    } else {
        response_length = jadren_http_router_respond_mode(
            connection->request, frame_length, response, sizeof(response),
            keep_alive);
    }
    if (response_length == 0 ||
        !jadren_http_session_send_all(connection, response,
                                       response_length)) {
        return -1;
    }
    if (session->chunked_mode || !keep_alive) {
        connection->request_length = 0;
        return -1;
    }
    connection->request_length = http_request_consume_prefix(
        connection->request, JADREN_HTTP_SESSION_REQUEST_CAPACITY,
        connection->request_length, frame_length);
    return 1;
}

static unsigned __int64 jadren_http_session_open_impl(
    unsigned __int64 listener, unsigned int max_connections,
    unsigned int max_header_bytes, unsigned int max_body_bytes,
    int tls_enabled, const char *certificate_data,
    unsigned __int64 certificate_length, const char *private_key_data,
    unsigned __int64 private_key_length, int chunked_mode) {
    int index;
    int connection_index;
    if (listener == 0 || max_connections == 0 ||
        max_connections > JADREN_HTTP_SESSION_CONNECTION_CAPACITY ||
        max_header_bytes < 64 || max_header_bytes > JADREN_HTTP_SESSION_HEADER_LIMIT ||
        max_body_bytes > JADREN_HTTP_SESSION_BODY_LIMIT ||
        max_header_bytes + max_body_bytes > JADREN_HTTP_SESSION_REQUEST_CAPACITY) {
        return 0;
    }
#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
    if (tls_enabled &&
        (certificate_data == 0 || private_key_data == 0 ||
         certificate_length == 0 ||
         certificate_length >= JADREN_HTTP_SESSION_TLS_PATH_CAPACITY ||
         private_key_length == 0 ||
         private_key_length >= JADREN_HTTP_SESSION_TLS_PATH_CAPACITY)) {
        return 0;
    }
#else
    (void)certificate_data;
    (void)certificate_length;
    (void)private_key_data;
    (void)private_key_length;
    if (tls_enabled) return 0;
#endif
    for (index = 0; index < JADREN_HTTP_SESSION_CAPACITY; index += 1) {
        JadrenHttpSession *session = jadren_http_sessions + index;
        if (session->active) {
            continue;
        }
        session->active = 1;
        session->listener = listener;
        session->max_connections = max_connections;
        session->max_header_bytes = max_header_bytes;
        session->max_body_bytes = max_body_bytes;
        session->cursor = 0;
        session->tls_enabled = tls_enabled;
        session->chunked_mode = chunked_mode;
        session->certificate_length = 0;
        session->private_key_length = 0;
#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
        if (tls_enabled) {
            unsigned int copy_index;
            session->certificate_length = certificate_length;
            session->private_key_length = private_key_length;
            for (copy_index = 0; copy_index < certificate_length; copy_index += 1) {
                session->certificate[copy_index] = certificate_data[copy_index];
            }
            for (copy_index = 0; copy_index < private_key_length; copy_index += 1) {
                session->private_key[copy_index] = private_key_data[copy_index];
            }
        }
#endif
        for (connection_index = 0;
             connection_index < JADREN_HTTP_SESSION_CONNECTION_CAPACITY;
             connection_index += 1) {
            session->connections[connection_index].active = 0;
            session->connections[connection_index].socket_token = 0;
            session->connections[connection_index].tls_token = 0;
            session->connections[connection_index].request_length = 0;
        }
        return (unsigned __int64)index + 1ULL;
    }
    return 0;
}

unsigned __int64 http_session_open(unsigned __int64 listener,
                                    unsigned int max_connections,
                                    unsigned int max_header_bytes,
                                    unsigned int max_body_bytes) {
    return jadren_http_session_open_impl(listener, max_connections,
                                          max_header_bytes, max_body_bytes,
                                          0, 0, 0, 0, 0, 0);
}

unsigned __int64 http_session_open_chunked(unsigned __int64 listener,
                                           unsigned int max_connections,
                                           unsigned int max_header_bytes,
                                           unsigned int max_body_bytes) {
    return jadren_http_session_open_impl(listener, max_connections,
                                          max_header_bytes, max_body_bytes,
                                          0, 0, 0, 0, 0, 1);
}

#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
unsigned __int64 http_session_open_tls(
    unsigned __int64 listener, unsigned int max_connections,
    unsigned int max_header_bytes, unsigned int max_body_bytes,
    const char *certificate_data, unsigned __int64 certificate_length,
    const char *private_key_data, unsigned __int64 private_key_length) {
    return jadren_http_session_open_impl(
        listener, max_connections, max_header_bytes, max_body_bytes, 1,
        certificate_data, certificate_length, private_key_data, private_key_length,
        0);
}

unsigned __int64 http_session_open_tls_chunked(
    unsigned __int64 listener, unsigned int max_connections,
    unsigned int max_header_bytes, unsigned int max_body_bytes,
    const char *certificate_data, unsigned __int64 certificate_length,
    const char *private_key_data, unsigned __int64 private_key_length) {
    return jadren_http_session_open_impl(
        listener, max_connections, max_header_bytes, max_body_bytes, 1,
        certificate_data, certificate_length, private_key_data, private_key_length,
        1);
}
#endif

/* Accepts one caller-owned connection slot without routing or reading it.
 * The returned opaque token is valid until http_session_close_connection or
 * http_session_close; callers must not mix it with http_session_step. */
unsigned __int64 http_session_accept(unsigned __int64 token,
                                     unsigned int timeout_ms) {
    JadrenHttpSession *session = jadren_http_session_find(token);
    unsigned __int64 accepted;
    int free_index = -1;
    int index;
    if (session == 0 || !session->active) return 0;
    for (index = 0; index < (int)session->max_connections; index += 1) {
        if (!session->connections[index].active) {
            free_index = index;
            break;
        }
    }
    if (free_index < 0 ||
        !jadren_http_session_wait_listener(session->listener,
                                           timeout_ms == 0 ? 1 : timeout_ms)) {
        return 0;
    }
    accepted = net_tcp_accept(session->listener);
    if (accepted == 0) return 0;
    session->connections[free_index].active = 1;
    session->connections[free_index].socket_token = accepted;
    session->connections[free_index].tls_token = 0;
#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
    if (session->tls_enabled) {
        session->connections[free_index].tls_token = net_tls_open_server(
            accepted, session->certificate, session->certificate_length,
            session->private_key, session->private_key_length);
        if (session->connections[free_index].tls_token == 0) {
            jadren_http_session_close_connection(
                &session->connections[free_index]);
            return 0;
        }
    }
#endif
    session->connections[free_index].request_length = 0;
    return jadren_http_session_connection_token(session,
                                                (unsigned int)free_index);
}

/* Reads one complete request into caller-owned output and consumes only that
 * frame. A short output, incomplete receive or malformed request returns 0
 * without publishing output; the caller explicitly closes the connection on
 * protocol failure. */
unsigned __int64 http_session_receive_request(
    unsigned __int64 connection_token, unsigned int timeout_ms,
    unsigned char *output_data, unsigned __int64 output_length) {
    JadrenHttpSession *session = 0;
    JadrenHttpSessionConnection *connection =
        jadren_http_session_find_connection(connection_token, &session);
    unsigned __int64 frame_length = 0;
    unsigned __int64 index;
    int state;
    if (connection == 0 || session == 0 || output_data == 0 || output_length == 0) {
        return 0;
    }
    state = jadren_http_session_read_frame(session, connection, timeout_ms,
                                           &frame_length);
    if (state != 1 || frame_length == 0 || frame_length > output_length) {
        return 0;
    }
    for (index = 0; index < frame_length; index += 1) {
        output_data[index] = connection->request[index];
    }
    connection->request_length = http_request_consume_prefix(
        connection->request, JADREN_HTTP_SESSION_REQUEST_CAPACITY,
        connection->request_length, frame_length);
    return frame_length;
}

/* Sends an explicit prefix of caller-owned response bytes. Framing, chunk
 * order and connection headers remain explicit in the caller. */
int http_session_send_prefix(unsigned __int64 connection_token,
                             const unsigned char *input_data,
                             unsigned __int64 input_capacity,
                             unsigned __int64 input_length) {
    JadrenHttpSessionConnection *connection =
        jadren_http_session_find_connection(connection_token, 0);
    if (connection == 0 || input_length > input_capacity ||
        (input_data == 0 && input_length > 0)) return 0;
    return jadren_http_session_send_all(connection, input_data, input_length);
}

int http_session_send(unsigned __int64 connection_token,
                      const unsigned char *input_data,
                      unsigned __int64 input_length) {
    return http_session_send_prefix(connection_token, input_data, input_length,
                                    input_length);
}

int http_session_close_connection(unsigned __int64 connection_token) {
    JadrenHttpSessionConnection *connection =
        jadren_http_session_find_connection(connection_token, 0);
    if (connection == 0) return 0;
    jadren_http_session_close_connection(connection);
    return 1;
}

unsigned int http_session_step(unsigned __int64 token, unsigned int timeout_ms) {
    JadrenHttpSession *session = jadren_http_session_find(token);
    unsigned int offset;
    if (session == 0 || !session->active) {
        return 0;
    }
    for (offset = 0; offset < session->max_connections; offset += 1) {
        unsigned int index = (session->cursor + offset) % session->max_connections;
        JadrenHttpSessionConnection *connection = &session->connections[index];
        if (!connection->active) {
            continue;
        }
        session->cursor = (index + 1) % session->max_connections;
        {
            int state = jadren_http_session_process_connection(session, connection,
                                                               timeout_ms);
            if (state < 0) {
                jadren_http_session_close_connection(connection);
                return 2;
            }
            if (state > 0) {
                return (unsigned int)state;
            }
        }
        break;
    }
    if (!jadren_http_session_wait_listener(session->listener,
                                           timeout_ms == 0 ? 1 : timeout_ms)) {
        return 0;
    }
    {
        unsigned __int64 accepted = net_tcp_accept(session->listener);
        int free_index = -1;
        int index;
        if (accepted == 0) {
            return 0;
        }
        for (index = 0; index < (int)session->max_connections; index += 1) {
            if (!session->connections[index].active) {
                free_index = index;
                break;
            }
        }
        if (free_index < 0) {
            (void)net_socket_close(accepted);
            return 2;
        }
        session->connections[free_index].active = 1;
        session->connections[free_index].socket_token = accepted;
        session->connections[free_index].tls_token = 0;
#if JADREN_FILE_RUNTIME_HAS_TLS_SUPPORT
        if (session->tls_enabled) {
            session->connections[free_index].tls_token = net_tls_open_server(
                accepted, session->certificate, session->certificate_length,
                session->private_key, session->private_key_length);
            if (session->connections[free_index].tls_token == 0) {
                jadren_http_session_close_connection(
                    &session->connections[free_index]);
                return 2;
            }
        }
#endif
        session->connections[free_index].request_length = 0;
        {
            int state = jadren_http_session_process_connection(
                session, &session->connections[free_index], timeout_ms);
            if (state < 0) {
            jadren_http_session_close_connection(&session->connections[free_index]);
            return 2;
            }
            if (state > 0) {
                return (unsigned int)state;
            }
        }
    }
    return 0;
}

int http_session_close(unsigned __int64 token) {
    JadrenHttpSession *session = jadren_http_session_find(token);
    int index;
    int success;
    if (session == 0 || !session->active) {
        return 0;
    }
    for (index = 0; index < JADREN_HTTP_SESSION_CONNECTION_CAPACITY; index += 1) {
        jadren_http_session_close_connection(&session->connections[index]);
    }
    success = net_socket_close(session->listener);
    session->active = 0;
    session->listener = 0;
    return success;
}
#endif

static int csv_escape_requires_quotes(const unsigned char *value_data,
                                      unsigned __int64 value_length) {
    unsigned __int64 index;
    for (index = 0; index < value_length; index += 1) {
        unsigned char value = value_data[index];
        if (value == ',' || value == '"' || value == '\r' || value == '\n') {
            return 1;
        }
    }
    return 0;
}

unsigned __int64 csv_escape(const char *value_data,
                            unsigned __int64 value_length,
                            unsigned char *output_data,
                            unsigned __int64 output_length) {
    unsigned __int64 required;
    unsigned __int64 index;
    unsigned __int64 offset;
    int quoted;
    if (value_data == 0 && value_length > 0) {
        return 0;
    }
    quoted = csv_escape_requires_quotes((const unsigned char *)value_data, value_length);
    required = quoted ? 2ULL : 0ULL;
    for (index = 0; index < value_length; index += 1) {
        unsigned __int64 addition = value_data[index] == '"' ? 2ULL : 1ULL;
        if (required > ((unsigned __int64)-1) - addition) {
            return 0;
        }
        required += addition;
    }
    if (output_data == 0 || output_length < required) {
        return 0;
    }
    offset = 0;
    if (quoted) {
        output_data[offset] = (unsigned char)'"';
        offset += 1;
    }
    for (index = 0; index < value_length; index += 1) {
        if (value_data[index] == '"') {
            output_data[offset] = (unsigned char)'"';
            offset += 1;
        }
        output_data[offset] = (unsigned char)value_data[index];
        offset += 1;
    }
    if (quoted) {
        output_data[offset] = (unsigned char)'"';
    }
    return required;
}

static unsigned __int64 json_escape_extra(unsigned char value) {
    if (value == '"' || value == '\\' || value == '\b' || value == '\f' ||
        value == '\n' || value == '\r' || value == '\t') {
        return 1ULL;
    }
    return value < 0x20U ? 5ULL : 0ULL;
}

static unsigned char json_escape_hex(unsigned char value) {
    return value < 10U ? (unsigned char)('0' + value)
                       : (unsigned char)('A' + value - 10U);
}

unsigned __int64 json_escape(const char *value_data,
                             unsigned __int64 value_length,
                             unsigned char *output_data,
                             unsigned __int64 output_length) {
    unsigned __int64 required = 2ULL;
    unsigned __int64 index;
    unsigned __int64 offset;
    if (value_data == 0 && value_length > 0) {
        return 0;
    }
    for (index = 0; index < value_length; index += 1) {
        unsigned __int64 addition = 1ULL + json_escape_extra((unsigned char)value_data[index]);
        if (required > ((unsigned __int64)-1) - addition) {
            return 0;
        }
        required += addition;
    }
    if (output_data == 0 || output_length < required) {
        return 0;
    }
    offset = 0;
    output_data[offset] = (unsigned char)'"';
    offset += 1;
    for (index = 0; index < value_length; index += 1) {
        unsigned char value = (unsigned char)value_data[index];
        if (value == '"' || value == '\\') {
            output_data[offset] = (unsigned char)'\\';
            offset += 1;
            output_data[offset] = value;
            offset += 1;
        } else if (value == '\b' || value == '\f' || value == '\n' ||
                   value == '\r' || value == '\t') {
            output_data[offset] = (unsigned char)'\\';
            offset += 1;
            output_data[offset] = value == '\b' ? (unsigned char)'b'
                                : value == '\f' ? (unsigned char)'f'
                                : value == '\n' ? (unsigned char)'n'
                                : value == '\r' ? (unsigned char)'r'
                                                 : (unsigned char)'t';
            offset += 1;
        } else if (value < 0x20U) {
            output_data[offset] = (unsigned char)'\\';
            output_data[offset + 1] = (unsigned char)'u';
            output_data[offset + 2] = (unsigned char)'0';
            output_data[offset + 3] = (unsigned char)'0';
            output_data[offset + 4] = json_escape_hex((unsigned char)(value >> 4));
            output_data[offset + 5] = json_escape_hex((unsigned char)(value & 0x0FU));
            offset += 6;
        } else {
            output_data[offset] = value;
            offset += 1;
        }
    }
    output_data[offset] = (unsigned char)'"';
    return required;
}

static unsigned __int64 json_escaped_length(const unsigned char *value_data,
                                            unsigned __int64 value_length) {
    unsigned __int64 required = 2ULL;
    unsigned __int64 index;
    if (value_data == 0 && value_length > 0) {
        return 0;
    }
    for (index = 0; index < value_length; index += 1) {
        unsigned __int64 addition = 1ULL + json_escape_extra(value_data[index]);
        if (required > ((unsigned __int64)-1) - addition) {
            return 0;
        }
        required += addition;
    }
    return required;
}

static unsigned __int64 json_write_escaped(const unsigned char *value_data,
                                           unsigned __int64 value_length,
                                           unsigned char *output_data,
                                           unsigned __int64 offset) {
    unsigned __int64 index;
    output_data[offset] = (unsigned char)'"';
    offset += 1;
    for (index = 0; index < value_length; index += 1) {
        unsigned char value = value_data[index];
        if (value == '"' || value == '\\') {
            output_data[offset] = (unsigned char)'\\';
            offset += 1;
            output_data[offset] = value;
            offset += 1;
        } else if (value == '\b' || value == '\f' || value == '\n' ||
                   value == '\r' || value == '\t') {
            output_data[offset] = (unsigned char)'\\';
            offset += 1;
            output_data[offset] = value == '\b' ? (unsigned char)'b'
                                : value == '\f' ? (unsigned char)'f'
                                : value == '\n' ? (unsigned char)'n'
                                : value == '\r' ? (unsigned char)'r'
                                                 : (unsigned char)'t';
            offset += 1;
        } else if (value < 0x20U) {
            output_data[offset] = (unsigned char)'\\';
            output_data[offset + 1] = (unsigned char)'u';
            output_data[offset + 2] = (unsigned char)'0';
            output_data[offset + 3] = (unsigned char)'0';
            output_data[offset + 4] = json_escape_hex((unsigned char)(value >> 4));
            output_data[offset + 5] = json_escape_hex((unsigned char)(value & 0x0FU));
            offset += 6;
        } else {
            output_data[offset] = value;
            offset += 1;
        }
    }
    output_data[offset] = (unsigned char)'"';
    return offset + 1;
}

unsigned __int64 json_object_field_string(const char *key_data,
                                          unsigned __int64 key_length,
                                          const char *value_data,
                                          unsigned __int64 value_length,
                                          unsigned char *output_data,
                                          unsigned __int64 output_length) {
    unsigned __int64 key_required = json_escaped_length(
        (const unsigned char *)key_data, key_length);
    unsigned __int64 value_required = json_escaped_length(
        (const unsigned char *)value_data, value_length);
    unsigned __int64 required;
    unsigned __int64 offset;
    if (key_required == 0 || value_required == 0 ||
        key_required > ((unsigned __int64)-1) - 1ULL ||
        key_required + 1ULL > ((unsigned __int64)-1) - value_required) {
        return 0;
    }
    required = key_required + 1ULL + value_required;
    if (output_data == 0 || output_length < required) {
        return 0;
    }
    offset = json_write_escaped((const unsigned char *)key_data, key_length,
                                output_data, 0);
    output_data[offset] = (unsigned char)':';
    offset += 1;
    (void)json_write_escaped((const unsigned char *)value_data, value_length,
                             output_data, offset);
    return required;
}

static unsigned __int64 json_object_field_raw(const char *key_data,
                                              unsigned __int64 key_length,
                                              const unsigned char *value_data,
                                              unsigned __int64 value_length,
                                              unsigned char *output_data,
                                              unsigned __int64 output_length) {
    unsigned __int64 key_required = json_escaped_length(
        (const unsigned char *)key_data, key_length);
    unsigned __int64 required;
    unsigned __int64 offset;
    unsigned __int64 index;
    if (key_required == 0 || (value_data == 0 && value_length > 0) ||
        key_required > ((unsigned __int64)-1) - 1ULL ||
        key_required + 1ULL > ((unsigned __int64)-1) - value_length) {
        return 0;
    }
    required = key_required + 1ULL + value_length;
    if (output_data == 0 || output_length < required) {
        return 0;
    }
    offset = json_write_escaped((const unsigned char *)key_data, key_length,
                                output_data, 0);
    output_data[offset] = (unsigned char)':';
    offset += 1;
    for (index = 0; index < value_length; index += 1) {
        output_data[offset + index] = value_data[index];
    }
    return required;
}

unsigned __int64 json_object_field_int(const char *key_data,
                                       unsigned __int64 key_length,
                                       long long value,
                                       unsigned char *output_data,
                                       unsigned __int64 output_length) {
    unsigned char value_data[20];
    unsigned __int64 value_length = format_int(value, value_data, sizeof(value_data));
    if (value_length == 0) {
        return 0;
    }
    return json_object_field_raw(key_data, key_length, value_data, value_length,
                                 output_data, output_length);
}

unsigned __int64 json_object_field_uint(const char *key_data,
                                        unsigned __int64 key_length,
                                        unsigned __int64 value,
                                        unsigned char *output_data,
                                        unsigned __int64 output_length) {
    unsigned char value_data[20];
    unsigned __int64 value_length = format_uint(value, value_data, sizeof(value_data));
    if (value_length == 0) {
        return 0;
    }
    return json_object_field_raw(key_data, key_length, value_data, value_length,
                                 output_data, output_length);
}

unsigned __int64 json_object_field_float(const char *key_data,
                                         unsigned __int64 key_length,
                                         double value,
                                         unsigned char *output_data,
                                         unsigned __int64 output_length) {
    unsigned char value_data[32];
    unsigned __int64 value_length = format_float(value, value_data, sizeof(value_data));
    if (value_length == 0 ||
        (value_length == 3 && (value_data[0] == (unsigned char)'n' ||
                               value_data[0] == (unsigned char)'i')) ||
        (value_length == 4 && value_data[0] == (unsigned char)'-')) {
        return 0;
    }
    return json_object_field_raw(key_data, key_length, value_data, value_length,
                                 output_data, output_length);
}

unsigned __int64 json_object_field_bool(const char *key_data,
                                        unsigned __int64 key_length,
                                        unsigned char value,
                                        unsigned char *output_data,
                                        unsigned __int64 output_length) {
    unsigned char value_data[5];
    unsigned __int64 value_length = format_bool(value, value_data, sizeof(value_data));
    if (value_length == 0) {
        return 0;
    }
    return json_object_field_raw(key_data, key_length, value_data, value_length,
                                 output_data, output_length);
}

/* Bounded flat-object JSON readers. They intentionally accept the exact
 * object/member shape emitted by the writer helpers above: whitespace,
 * primitive values and escaped strings are supported; nested objects/arrays
 * are rejected so a malformed or ambiguous state file cannot be partially
 * interpreted. */
static int json_read_skip_ws(const unsigned char *data,
                             unsigned __int64 length,
                             unsigned __int64 *index) {
    while (*index < length && (data[*index] == ' ' || data[*index] == '\t' ||
                               data[*index] == '\r' || data[*index] == '\n')) {
        *index += 1;
    }
    return *index < length;
}

static int json_read_string_end(const unsigned char *data,
                                unsigned __int64 length,
                                unsigned __int64 start,
                                unsigned __int64 *end) {
    unsigned __int64 index;
    if (start >= length || data[start] != '"') {
        return 0;
    }
    index = start + 1;
    while (index < length) {
        if (data[index] == '"') {
            *end = index + 1;
            return 1;
        }
        if (data[index] < 0x20U) {
            return 0;
        }
        if (data[index] == '\\') {
            index += 1;
            if (index >= length || (data[index] != '"' && data[index] != '\\' &&
                                   data[index] != '/' && data[index] != 'b' &&
                                   data[index] != 'f' && data[index] != 'n' &&
                                   data[index] != 'r' && data[index] != 't')) {
                return 0;
            }
        }
        index += 1;
    }
    return 0;
}

static int json_read_key_equals(const unsigned char *data,
                                unsigned __int64 start,
                                unsigned __int64 end,
                                const char *key_data,
                                unsigned __int64 key_length) {
    unsigned __int64 index;
    if (end <= start + 1 || end - start - 2 != key_length ||
        (key_data == 0 && key_length > 0)) {
        return 0;
    }
    for (index = 0; index < key_length; index += 1) {
        if (data[start + 1 + index] == '\\' ||
            data[start + 1 + index] != (unsigned char)key_data[index]) {
            return 0;
        }
    }
    return 1;
}

static int json_read_find_value(const unsigned char *data,
                                unsigned __int64 length,
                                const char *key_data,
                                unsigned __int64 key_length,
                                unsigned __int64 *value_start,
                                unsigned __int64 *value_end) {
    unsigned __int64 index = 0;
    unsigned __int64 key_start;
    unsigned __int64 key_end;
    unsigned __int64 start;
    unsigned __int64 end;
    int matched;
    if (data == 0 || length == 0 || (key_data == 0 && key_length > 0) ||
        !json_read_skip_ws(data, length, &index) || data[index] != '{') {
        return 0;
    }
    index += 1;
    while (json_read_skip_ws(data, length, &index)) {
        if (data[index] == '}') {
            return 0;
        }
        key_start = index;
        if (!json_read_string_end(data, length, key_start, &key_end)) {
            return 0;
        }
        matched = json_read_key_equals(data, key_start, key_end, key_data, key_length);
        index = key_end;
        if (!json_read_skip_ws(data, length, &index) || data[index] != ':') {
            return 0;
        }
        index += 1;
        if (!json_read_skip_ws(data, length, &index)) {
            return 0;
        }
        start = index;
        if (data[index] == '"') {
            if (!json_read_string_end(data, length, start, &end)) {
                return 0;
            }
        } else {
            if (data[index] == '{' || data[index] == '[') {
                return 0;
            }
            while (index < length && data[index] != ',' && data[index] != '}') {
                index += 1;
            }
            end = index;
            while (end > start && (data[end - 1] == ' ' || data[end - 1] == '\t' ||
                                   data[end - 1] == '\r' || data[end - 1] == '\n')) {
                end -= 1;
            }
            if (end == start) {
                return 0;
            }
        }
        if (matched) {
            *value_start = start;
            *value_end = end;
            return 1;
        }
        index = end;
        if (!json_read_skip_ws(data, length, &index)) {
            return 0;
        }
        if (data[index] == ',') {
            index += 1;
            continue;
        }
        return 0;
    }
    return 0;
}

static unsigned __int64 json_read_string_length(const unsigned char *data,
                                                unsigned __int64 start,
                                                unsigned __int64 end) {
    unsigned __int64 index;
    unsigned __int64 length = 0;
    if (end <= start + 1 || data[start] != '"' || data[end - 1] != '"') {
        return 0;
    }
    for (index = start + 1; index + 1 < end; index += 1) {
        if (data[index] == '\\') {
            index += 1;
            if (index + 1 >= end || data[index] == 'u') {
                return 0;
            }
        }
        length += 1;
    }
    return length;
}

static unsigned __int64 json_read_string_copy(const unsigned char *data,
                                              unsigned __int64 start,
                                              unsigned __int64 end,
                                              unsigned char *output_data) {
    unsigned __int64 index;
    unsigned __int64 offset = 0;
    for (index = start + 1; index + 1 < end; index += 1) {
        if (data[index] == '\\') {
            index += 1;
            if (data[index] == 'b') output_data[offset] = '\b';
            else if (data[index] == 'f') output_data[offset] = '\f';
            else if (data[index] == 'n') output_data[offset] = '\n';
            else if (data[index] == 'r') output_data[offset] = '\r';
            else if (data[index] == 't') output_data[offset] = '\t';
            else output_data[offset] = data[index];
        } else {
            output_data[offset] = data[index];
        }
        offset += 1;
    }
    return offset;
}

unsigned __int64 json_object_read_string(const unsigned char *input_data,
                                         unsigned __int64 input_length,
                                         const char *key_data,
                                         unsigned __int64 key_length,
                                         unsigned char *output_data,
                                         unsigned __int64 output_length) {
    unsigned __int64 start;
    unsigned __int64 end;
    unsigned __int64 required;
    if (!json_read_find_value(input_data, input_length, key_data, key_length,
                              &start, &end) || input_data[start] != '"') {
        return 0;
    }
    required = json_read_string_length(input_data, start, end);
    if (required == 0 || output_data == 0 || output_length < required) {
        return 0;
    }
    return json_read_string_copy(input_data, start, end, output_data);
}

static int json_read_int_value(const unsigned char *data,
                               unsigned __int64 start,
                               unsigned __int64 end,
                               long long *result) {
    unsigned __int64 index = start;
    unsigned __int64 magnitude = 0;
    unsigned __int64 limit;
    int negative = 0;
    if (index < end && data[index] == '-') {
        negative = 1;
        index += 1;
    }
    if (index >= end || data[index] < '0' || data[index] > '9') {
        return 0;
    }
    limit = negative ? 0x8000000000000000ULL : 0x7FFFFFFFFFFFFFFFULL;
    while (index < end) {
        unsigned __int64 digit;
        if (data[index] < '0' || data[index] > '9') {
            return 0;
        }
        digit = (unsigned __int64)(data[index] - '0');
        if (magnitude > (limit - digit) / 10ULL) {
            return 0;
        }
        magnitude = magnitude * 10ULL + digit;
        index += 1;
    }
    if (negative && magnitude == 0x8000000000000000ULL) {
        *result = (-9223372036854775807LL - 1LL);
    } else {
        *result = negative ? -(long long)magnitude : (long long)magnitude;
    }
    return 1;
}

static int json_read_uint_value(const unsigned char *data,
                                unsigned __int64 start,
                                unsigned __int64 end,
                                unsigned __int64 *result) {
    unsigned __int64 index = start;
    unsigned __int64 value = 0;
    if (index >= end) return 0;
    while (index < end) {
        unsigned __int64 digit;
        if (data[index] < '0' || data[index] > '9') return 0;
        digit = (unsigned __int64)(data[index] - '0');
        if (value > (0xFFFFFFFFFFFFFFFFULL - digit) / 10ULL) return 0;
        value = value * 10ULL + digit;
        index += 1;
    }
    *result = value;
    return 1;
}

static int json_read_float_value(const unsigned char *data,
                                 unsigned __int64 start,
                                 unsigned __int64 end,
                                 double *result) {
    union {
        double value;
        unsigned long long bits;
    } representation;
    unsigned __int64 index = start;
    unsigned int fractional_digits = 0;
    unsigned int exponent_value = 0;
    double value = 0.0;
    int negative = 0;
    int exponent_negative = 0;
    if (data == 0 || result == 0 || start >= end) return 0;
    if (data[index] == '-') {
        negative = 1;
        index += 1;
    }
    if (index >= end || data[index] < '0' || data[index] > '9') return 0;
    if (data[index] == '0') {
        index += 1;
    } else {
        while (index < end && data[index] >= '0' && data[index] <= '9') {
            value = value * 10.0 + (double)(data[index] - '0');
            index += 1;
        }
    }
    if (index < end && data[index] == '.') {
        index += 1;
        if (index >= end || data[index] < '0' || data[index] > '9') return 0;
        while (index < end && data[index] >= '0' && data[index] <= '9') {
            value = value * 10.0 + (double)(data[index] - '0');
            fractional_digits += 1;
            index += 1;
        }
    }
    while (fractional_digits > 0) {
        value /= 10.0;
        fractional_digits -= 1;
    }
    if (index < end && (data[index] == 'e' || data[index] == 'E')) {
        index += 1;
        if (index < end && (data[index] == '+' || data[index] == '-')) {
            exponent_negative = data[index] == '-';
            index += 1;
        }
        if (index >= end || data[index] < '0' || data[index] > '9') return 0;
        while (index < end && data[index] >= '0' && data[index] <= '9') {
            if (exponent_value < 401U) {
                exponent_value = exponent_value * 10U + (unsigned int)(data[index] - '0');
                if (exponent_value > 401U) exponent_value = 401U;
            }
            index += 1;
        }
    }
    if (index != end) return 0;
    while (exponent_value > 0) {
        if (exponent_negative) value /= 10.0;
        else value *= 10.0;
        exponent_value -= 1;
    }
    if (negative) value = -value;
    representation.value = value;
    if (((representation.bits >> 52) & 0x7FFULL) == 0x7FFULL) return 0;
    *result = value;
    return 1;
}

long long json_object_read_int(const unsigned char *input_data,
                               unsigned __int64 input_length,
                               const char *key_data,
                               unsigned __int64 key_length) {
    unsigned __int64 start;
    unsigned __int64 end;
    long long result = 0;
    if (json_read_find_value(input_data, input_length, key_data, key_length,
                             &start, &end)) {
        (void)json_read_int_value(input_data, start, end, &result);
    }
    return result;
}

unsigned __int64 json_object_read_uint(const unsigned char *input_data,
                                       unsigned __int64 input_length,
                                       const char *key_data,
                                       unsigned __int64 key_length) {
    unsigned __int64 start;
    unsigned __int64 end;
    unsigned __int64 result = 0;
    if (json_read_find_value(input_data, input_length, key_data, key_length,
                             &start, &end)) {
        (void)json_read_uint_value(input_data, start, end, &result);
    }
    return result;
}

double json_object_read_float(const unsigned char *input_data,
                              unsigned __int64 input_length,
                              const char *key_data,
                              unsigned __int64 key_length) {
    unsigned __int64 start;
    unsigned __int64 end;
    double result = 0.0;
    if (json_read_find_value(input_data, input_length, key_data, key_length,
                             &start, &end)) {
        (void)json_read_float_value(input_data, start, end, &result);
    }
    return result;
}

unsigned char json_object_read_bool(const unsigned char *input_data,
                                    unsigned __int64 input_length,
                                    const char *key_data,
                                    unsigned __int64 key_length) {
    unsigned __int64 start;
    unsigned __int64 end;
    if (!json_read_find_value(input_data, input_length, key_data, key_length,
                              &start, &end)) {
        return 0;
    }
    if (end - start == 4 && input_data[start] == 't' && input_data[start + 1] == 'r' &&
        input_data[start + 2] == 'u' && input_data[start + 3] == 'e') {
        return 1;
    }
    return 0;
}

int json_object_read_string_exact(const unsigned char *input_data,
                                  unsigned __int64 input_length,
                                  const char *key_data,
                                  unsigned __int64 key_length,
                                  unsigned char *output_data,
                                  unsigned __int64 output_length,
                                  unsigned __int64 *output_size,
                                  unsigned __int64 output_size_capacity) {
    unsigned __int64 start;
    unsigned __int64 end;
    unsigned __int64 required;
    if (output_size == 0 || output_size_capacity == 0 ||
        !json_read_find_value(input_data, input_length, key_data, key_length,
                              &start, &end) || input_data[start] != '"' ||
        !json_read_string_end(input_data, input_length, start, &end)) {
        return 0;
    }
    required = json_read_string_length(input_data, start, end);
    if ((required > 0 && (output_data == 0 || output_length < required))) return 0;
    if (required > 0) (void)json_read_string_copy(input_data, start, end, output_data);
    output_size[0] = required;
    return 1;
}

int json_object_read_int_exact(const unsigned char *input_data,
                               unsigned __int64 input_length,
                               const char *key_data,
                               unsigned __int64 key_length,
                               long long *output_data,
                               unsigned __int64 output_length) {
    unsigned __int64 start;
    unsigned __int64 end;
    long long value;
    if (output_data == 0 || output_length == 0 ||
        !json_read_find_value(input_data, input_length, key_data, key_length, &start, &end) ||
        !json_read_int_value(input_data, start, end, &value)) return 0;
    output_data[0] = value;
    return 1;
}

int json_object_read_uint_exact(const unsigned char *input_data,
                                unsigned __int64 input_length,
                                const char *key_data,
                                unsigned __int64 key_length,
                                unsigned __int64 *output_data,
                                unsigned __int64 output_length) {
    unsigned __int64 start;
    unsigned __int64 end;
    unsigned __int64 value;
    if (output_data == 0 || output_length == 0 ||
        !json_read_find_value(input_data, input_length, key_data, key_length, &start, &end) ||
        !json_read_uint_value(input_data, start, end, &value)) return 0;
    output_data[0] = value;
    return 1;
}

int json_object_read_float_exact(const unsigned char *input_data,
                                 unsigned __int64 input_length,
                                 const char *key_data,
                                 unsigned __int64 key_length,
                                 double *output_data,
                                 unsigned __int64 output_length) {
    unsigned __int64 start;
    unsigned __int64 end;
    double value;
    if (output_data == 0 || output_length == 0 ||
        !json_read_find_value(input_data, input_length, key_data, key_length, &start, &end) ||
        !json_read_float_value(input_data, start, end, &value)) return 0;
    output_data[0] = value;
    return 1;
}

int json_object_read_bool_exact(const unsigned char *input_data,
                                unsigned __int64 input_length,
                                const char *key_data,
                                unsigned __int64 key_length,
                                unsigned char *output_data,
                                unsigned __int64 output_length) {
    unsigned __int64 start;
    unsigned __int64 end;
    unsigned char value;
    if (output_data == 0 || output_length == 0 ||
        !json_read_find_value(input_data, input_length, key_data, key_length, &start, &end)) return 0;
    if (end - start == 4 && input_data[start] == 't' && input_data[start + 1] == 'r' &&
        input_data[start + 2] == 'u' && input_data[start + 3] == 'e') {
        value = 1;
    } else if (end - start == 5 && input_data[start] == 'f' && input_data[start + 1] == 'a' &&
               input_data[start + 2] == 'l' && input_data[start + 3] == 's' &&
               input_data[start + 4] == 'e') {
        value = 0;
    } else {
        return 0;
    }
    output_data[0] = value;
    return 1;
}

/* Text parsing is intentionally caller-owned: input_length selects a valid
 * prefix of the byte slice and a one-element output slice receives a value
 * only after the full input has passed its grammar and overflow checks. */
int parse_int(const unsigned char *input_data, unsigned __int64 input_capacity,
              unsigned __int64 input_length, long long *output_data,
              unsigned __int64 output_length) {
    unsigned __int64 index = 0;
    unsigned __int64 value = 0;
    unsigned __int64 limit = 0x7FFFFFFFFFFFFFFFULL;
    int negative = 0;
    if (input_data == 0 || input_length == 0 || input_length > input_capacity || output_data == 0 ||
        output_length == 0) return 0;
    if (input_data[0] == '-') {
        negative = 1;
        index = 1;
        limit = 0x8000000000000000ULL;
    } else if (input_data[0] == '+') {
        index = 1;
    }
    if (index >= input_length) return 0;
    for (; index < input_length; index += 1) {
        unsigned char digit = input_data[index];
        if (digit < '0' || digit > '9') return 0;
        digit = (unsigned char)(digit - '0');
        if (value > (limit - digit) / 10ULL) return 0;
        value = value * 10ULL + digit;
    }
    if (negative && value == 0x8000000000000000ULL) {
        output_data[0] = (-9223372036854775807LL - 1LL);
    } else {
        output_data[0] = negative ? -(long long)value : (long long)value;
    }
    return 1;
}

int parse_uint(const unsigned char *input_data, unsigned __int64 input_capacity,
               unsigned __int64 input_length, unsigned __int64 *output_data,
               unsigned __int64 output_length) {
    unsigned __int64 index = 0;
    unsigned __int64 value = 0;
    if (input_data == 0 || input_length == 0 || input_length > input_capacity || output_data == 0 ||
        output_length == 0) return 0;
    if (input_data[0] == '+') index = 1;
    if (index >= input_length) return 0;
    for (; index < input_length; index += 1) {
        unsigned char digit = input_data[index];
        if (digit < '0' || digit > '9') return 0;
        digit = (unsigned char)(digit - '0');
        if (value > (0xFFFFFFFFFFFFFFFFULL - digit) / 10ULL) return 0;
        value = value * 10ULL + digit;
    }
    output_data[0] = value;
    return 1;
}

int parse_float(const unsigned char *input_data, unsigned __int64 input_capacity,
                unsigned __int64 input_length, double *output_data,
                unsigned __int64 output_length) {
    double value = 0.0;
    if (input_data == 0 || input_length == 0 || input_length > input_capacity || output_data == 0 ||
        output_length == 0 ||
        !json_read_float_value(input_data, 0, input_length, &value)) return 0;
    output_data[0] = value;
    return 1;
}

int parse_bool(const unsigned char *input_data, unsigned __int64 input_capacity,
               unsigned __int64 input_length, unsigned char *output_data,
               unsigned __int64 output_length) {
    unsigned char value;
    if (input_data == 0 || input_length > input_capacity || output_data == 0 || output_length == 0) return 0;
    if (input_length == 4 && input_data[0] == 't' && input_data[1] == 'r' &&
        input_data[2] == 'u' && input_data[3] == 'e') {
        value = 1;
    } else if (input_length == 5 && input_data[0] == 'f' && input_data[1] == 'a' &&
               input_data[2] == 'l' && input_data[3] == 's' && input_data[4] == 'e') {
        value = 0;
    } else {
        return 0;
    }
    output_data[0] = value;
    return 1;
}

#define JADREN_APP_STATE_MAX_ENTRIES 32
#define JADREN_APP_STATE_KEY_MAX 64
#define JADREN_APP_STATE_TEXT_MAX 256
#define JADREN_APP_STATE_DOCUMENT_MAX 16384

enum {
    JADREN_APP_STATE_INT = 1,
    JADREN_APP_STATE_UINT = 2,
    JADREN_APP_STATE_BOOL = 3,
    JADREN_APP_STATE_TEXT = 4,
    JADREN_APP_STATE_FLOAT = 5
};

typedef struct JadrenAppStateEntry {
    unsigned char used;
    unsigned char kind;
    unsigned __int64 key_length;
    unsigned char key[JADREN_APP_STATE_KEY_MAX];
    long long int_value;
    unsigned __int64 uint_value;
    unsigned char bool_value;
    double float_value;
    unsigned __int64 text_length;
    unsigned char text[JADREN_APP_STATE_TEXT_MAX];
} JadrenAppStateEntry;

static JadrenAppStateEntry jadren_app_state[JADREN_APP_STATE_MAX_ENTRIES];
static JadrenAppStateEntry jadren_app_state_backup[JADREN_APP_STATE_MAX_ENTRIES];
static unsigned char jadren_app_state_transaction_active;
static unsigned char jadren_app_data_transaction_active;
static unsigned __int64 jadren_app_state_revision;

static void app_state_bump_revision(void) {
    if (jadren_app_state_revision != 0xFFFFFFFFFFFFFFFFULL) {
        jadren_app_state_revision += 1;
    }
}

unsigned __int64 app_state_revision(void) {
    return jadren_app_state_revision;
}

static int app_state_key_is_safe(const unsigned char *key_data,
                                 unsigned __int64 key_length) {
    unsigned __int64 index;
    if (key_data == 0 || key_length == 0 || key_length > JADREN_APP_STATE_KEY_MAX) {
        return 0;
    }
    for (index = 0; index < key_length; index += 1) {
        unsigned char value = key_data[index];
        if (value < 0x20U || value == (unsigned char)'"' ||
            value == (unsigned char)'\\') {
            return 0;
        }
    }
    return 1;
}

static int app_state_find_in(JadrenAppStateEntry *entries,
                             const unsigned char *key_data,
                             unsigned __int64 key_length) {
    unsigned int entry_index;
    unsigned __int64 index;
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        if (!entries[entry_index].used || entries[entry_index].key_length != key_length) {
            continue;
        }
        for (index = 0; index < key_length; index += 1) {
            if (entries[entry_index].key[index] != key_data[index]) {
                break;
            }
        }
        if (index == key_length) {
            return (int)entry_index;
        }
    }
    return -1;
}

static int app_state_slot_in(JadrenAppStateEntry *entries,
                             const unsigned char *key_data,
                             unsigned __int64 key_length) {
    int found = app_state_find_in(entries, key_data, key_length);
    unsigned int entry_index;
    unsigned __int64 index;
    if (found >= 0) {
        return found;
    }
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        if (!entries[entry_index].used) {
            entries[entry_index].used = 1;
            entries[entry_index].kind = 0;
            entries[entry_index].key_length = key_length;
            for (index = 0; index < key_length; index += 1) {
                entries[entry_index].key[index] = key_data[index];
            }
            return (int)entry_index;
        }
    }
    return -1;
}

static void app_state_clear_entry(JadrenAppStateEntry *entry) {
    unsigned __int64 byte_index;
    entry->used = 0;
    entry->kind = 0;
    entry->key_length = 0;
    entry->int_value = 0;
    entry->uint_value = 0;
    entry->bool_value = 0;
    entry->float_value = 0.0;
    entry->text_length = 0;
    for (byte_index = 0; byte_index < JADREN_APP_STATE_KEY_MAX; byte_index += 1) {
        entry->key[byte_index] = 0;
    }
    for (byte_index = 0; byte_index < JADREN_APP_STATE_TEXT_MAX; byte_index += 1) {
        entry->text[byte_index] = 0;
    }
}

static void app_state_copy_entry(JadrenAppStateEntry *destination,
                                 const JadrenAppStateEntry *source) {
    unsigned __int64 byte_index;
    destination->used = source->used;
    destination->kind = source->kind;
    destination->key_length = source->key_length;
    destination->int_value = source->int_value;
    destination->uint_value = source->uint_value;
    destination->bool_value = source->bool_value;
    destination->float_value = source->float_value;
    destination->text_length = source->text_length;
    for (byte_index = 0; byte_index < JADREN_APP_STATE_KEY_MAX; byte_index += 1) {
        destination->key[byte_index] = source->key[byte_index];
    }
    for (byte_index = 0; byte_index < JADREN_APP_STATE_TEXT_MAX; byte_index += 1) {
        destination->text[byte_index] = source->text[byte_index];
    }
}

static void app_state_clear_entries(JadrenAppStateEntry *entries) {
    unsigned int entry_index;
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        app_state_clear_entry(&entries[entry_index]);
    }
}

int app_state_tx_begin(void) {
    unsigned int entry_index;
    if (jadren_app_state_transaction_active || jadren_app_data_transaction_active) return 0;
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        app_state_copy_entry(&jadren_app_state_backup[entry_index], &jadren_app_state[entry_index]);
    }
    jadren_app_state_transaction_active = 1;
    return 1;
}

int app_state_tx_commit(void) {
    if (!jadren_app_state_transaction_active) return 0;
    app_state_clear_entries(jadren_app_state_backup);
    jadren_app_state_transaction_active = 0;
    return 1;
}

int app_state_tx_rollback(void) {
    unsigned int entry_index;
    if (!jadren_app_state_transaction_active) return 0;
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        app_state_copy_entry(&jadren_app_state[entry_index], &jadren_app_state_backup[entry_index]);
    }
    app_state_clear_entries(jadren_app_state_backup);
    jadren_app_state_transaction_active = 0;
    app_state_bump_revision();
    app_data_refresh_bound_ui();
    return 1;
}

void app_state_clear(void) {
    app_state_clear_entries(jadren_app_state);
    app_state_bump_revision();
}

int app_state_count(void) {
    unsigned int entry_index;
    int count = 0;
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        if (jadren_app_state[entry_index].used) count += 1;
    }
    return count;
}

int app_state_type_at(int key_index) {
    unsigned int entry_index;
    int visible_index = 0;
    if (key_index < 0) return 0;
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        if (!jadren_app_state[entry_index].used) continue;
        if (visible_index == key_index) return jadren_app_state[entry_index].kind;
        visible_index += 1;
    }
    return 0;
}

int app_state_exists(const char *key_data, unsigned __int64 key_length) {
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) return 0;
    return app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length) >= 0;
}

int app_state_remove(const char *key_data, unsigned __int64 key_length) {
    int slot;
    unsigned int entry_index;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) return 0;
    slot = app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0) return 0;
    for (entry_index = (unsigned int)slot;
         entry_index + 1 < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        if (!jadren_app_state[entry_index + 1].used) {
            app_state_clear_entry(&jadren_app_state[entry_index]);
            app_state_bump_revision();
            return 1;
        }
        app_state_copy_entry(&jadren_app_state[entry_index],
                             &jadren_app_state[entry_index + 1]);
    }
    app_state_clear_entry(&jadren_app_state[JADREN_APP_STATE_MAX_ENTRIES - 1]);
    app_state_bump_revision();
    return 1;
}

/* Remove one state entry only when the caller's equality-only revision is
 * still current. Stale, missing, invalid, or empty-key calls leave the model
 * and revision unchanged. This is process-local coordination, not a
 * cross-thread atomic or persistence primitive. */
int app_state_remove_if_revision(const char *key_data,
                                 unsigned __int64 key_length,
                                 unsigned __int64 expected_revision) {
    if (app_state_revision() != expected_revision) return 0;
    return app_state_remove(key_data, key_length);
}

unsigned __int64 app_state_read_key(int key_index, unsigned char *output_data,
                                    unsigned __int64 output_length) {
    unsigned int entry_index;
    int visible_index = 0;
    if (key_index < 0 || output_data == 0) return 0;
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        JadrenAppStateEntry *entry = &jadren_app_state[entry_index];
        unsigned __int64 byte_index;
        if (!entry->used) continue;
        if (visible_index != key_index) {
            visible_index += 1;
            continue;
        }
        if (output_length < entry->key_length) return 0;
        for (byte_index = 0; byte_index < entry->key_length; byte_index += 1) {
            output_data[byte_index] = entry->key[byte_index];
        }
        return entry->key_length;
    }
    return 0;
}

int app_state_read_key_exact(int key_index, unsigned char *output_data,
                             unsigned __int64 output_length,
                             unsigned __int64 *output_key_length,
                             unsigned __int64 output_key_length_capacity) {
    unsigned int entry_index;
    int visible_index = 0;
    unsigned __int64 byte_index;
    unsigned __int64 key_length;
    if (key_index < 0 || output_key_length == 0 || output_key_length_capacity == 0) return 0;
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        JadrenAppStateEntry *entry = &jadren_app_state[entry_index];
        if (!entry->used) continue;
        if (visible_index != key_index) {
            visible_index += 1;
            continue;
        }
        key_length = entry->key_length;
        if ((output_data == 0 && key_length > 0) || output_length < key_length) return 0;
        for (byte_index = 0; byte_index < key_length; byte_index += 1) {
            output_data[byte_index] = entry->key[byte_index];
        }
        output_key_length[0] = key_length;
        return 1;
    }
    return 0;
}

int app_state_set_int(const char *key_data, unsigned __int64 key_length,
                      long long value) {
    int slot;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_INT;
    jadren_app_state[slot].int_value = value;
    app_state_bump_revision();
    return 1;
}

/* Replace a signed value only when the caller's equality-only revision is
 * still current. Stale, invalid, or unrepresentable calls leave the entry
 * and revision unchanged. This is process-local coordination, not a
 * cross-thread atomic or persistence primitive. */
int app_state_set_int_if_revision(const char *key_data,
                                  unsigned __int64 key_length,
                                  long long value,
                                  unsigned __int64 expected_revision) {
    int slot;
    if (app_state_revision() != expected_revision) {
        return 0;
    }
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_INT;
    jadren_app_state[slot].int_value = value;
    app_state_bump_revision();
    return 1;
}

/* Apply a bounded signed delta in one native state operation. A missing key
 * starts at zero; an existing key must already be Int64. Overflow is rejected
 * before the entry or revision is changed. This is an explicit caller boundary
 * for counters, not a cross-thread atomic or persistence primitive. */
int app_state_add_int(const char *key_data, unsigned __int64 key_length,
                      long long delta) {
    int slot;
    long long current;
    const long long max_value = 9223372036854775807LL;
    const long long min_value = (-9223372036854775807LL - 1LL);
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    if (jadren_app_state[slot].kind == 0) {
        current = 0;
    } else if (jadren_app_state[slot].kind == JADREN_APP_STATE_INT) {
        current = jadren_app_state[slot].int_value;
    } else {
        return 0;
    }
    if ((delta > 0 && current > max_value - delta) ||
        (delta < 0 && current < min_value - delta)) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_INT;
    jadren_app_state[slot].int_value = current + delta;
    app_state_bump_revision();
    return 1;
}

/* Apply a signed delta only when the caller's equality-only revision is still
 * current. A rejected stale, type-mismatched, invalid, or overflowing call
 * leaves both the entry and revision unchanged. This is process-local caller
 * coordination, not a cross-thread atomic or persistence primitive. */
int app_state_add_int_if_revision(const char *key_data,
                                  unsigned __int64 key_length,
                                  long long delta,
                                  unsigned __int64 expected_revision) {
    int slot;
    long long current;
    const long long max_value = 9223372036854775807LL;
    const long long min_value = (-9223372036854775807LL - 1LL);
    if (app_state_revision() != expected_revision) {
        return 0;
    }
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    if (jadren_app_state[slot].kind == 0) {
        current = 0;
    } else if (jadren_app_state[slot].kind == JADREN_APP_STATE_INT) {
        current = jadren_app_state[slot].int_value;
    } else {
        return 0;
    }
    if ((delta > 0 && current > max_value - delta) ||
        (delta < 0 && current < min_value - delta)) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_INT;
    jadren_app_state[slot].int_value = current + delta;
    app_state_bump_revision();
    return 1;
}

long long app_state_get_int(const char *key_data, unsigned __int64 key_length) {
    int slot = app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    if (jadren_app_state[slot].kind == JADREN_APP_STATE_INT) {
        return jadren_app_state[slot].int_value;
    }
    if (jadren_app_state[slot].kind == JADREN_APP_STATE_UINT &&
        jadren_app_state[slot].uint_value <= 0x7FFFFFFFFFFFFFFFULL) {
        return (long long)jadren_app_state[slot].uint_value;
    }
    return 0;
}

int app_state_set_uint(const char *key_data, unsigned __int64 key_length,
                       unsigned __int64 value) {
    int slot;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_UINT;
    jadren_app_state[slot].uint_value = value;
    app_state_bump_revision();
    return 1;
}

/* Replace an unsigned value only when the caller's equality-only revision is
 * current. Stale or invalid calls leave the entry and revision unchanged. */
int app_state_set_uint_if_revision(const char *key_data,
                                   unsigned __int64 key_length,
                                   unsigned __int64 value,
                                   unsigned __int64 expected_revision) {
    int slot;
    if (app_state_revision() != expected_revision) return 0;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) return 0;
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) return 0;
    jadren_app_state[slot].kind = JADREN_APP_STATE_UINT;
    jadren_app_state[slot].uint_value = value;
    app_state_bump_revision();
    return 1;
}

/* Apply a bounded unsigned delta in one native state operation. A missing
 * key starts at zero; an existing key must be UInt64. Overflow is rejected
 * before the entry or revision is changed. This is a caller-owned counter
 * boundary, not a cross-thread atomic or persistence primitive. */
int app_state_add_uint(const char *key_data, unsigned __int64 key_length,
                       unsigned __int64 delta) {
    int slot;
    unsigned __int64 current;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    if (jadren_app_state[slot].kind == 0) {
        current = 0;
    } else if (jadren_app_state[slot].kind == JADREN_APP_STATE_UINT) {
        current = jadren_app_state[slot].uint_value;
    } else {
        return 0;
    }
    if (current > 0xFFFFFFFFFFFFFFFFULL - delta) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_UINT;
    jadren_app_state[slot].uint_value = current + delta;
    app_state_bump_revision();
    return 1;
}

/* Apply an unsigned delta only when the caller's equality-only revision is
 * still current. Stale, type-mismatched, invalid, and overflowing calls
 * leave both the entry and revision unchanged. */
int app_state_add_uint_if_revision(const char *key_data,
                                   unsigned __int64 key_length,
                                   unsigned __int64 delta,
                                   unsigned __int64 expected_revision) {
    int slot;
    unsigned __int64 current;
    if (app_state_revision() != expected_revision) {
        return 0;
    }
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    if (jadren_app_state[slot].kind == 0) {
        current = 0;
    } else if (jadren_app_state[slot].kind == JADREN_APP_STATE_UINT) {
        current = jadren_app_state[slot].uint_value;
    } else {
        return 0;
    }
    if (current > 0xFFFFFFFFFFFFFFFFULL - delta) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_UINT;
    jadren_app_state[slot].uint_value = current + delta;
    app_state_bump_revision();
    return 1;
}

/* Apply a finite floating-point delta in one native state operation. A
 * missing key starts at zero; an existing key must already be Float64. The
 * finite input and result are validated before the entry or revision changes.
 * This is a caller-owned numeric primitive, not a persistence or atomic API. */
int app_state_add_float(const char *key_data, unsigned __int64 key_length,
                        double delta) {
    union {
        double value;
        unsigned long long bits;
    } delta_representation, sum_representation;
    unsigned char encoded[32];
    int slot;
    double current;
    double sum;
    delta_representation.value = delta;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length) ||
        ((delta_representation.bits >> 52) & 0x7FFULL) == 0x7FFULL) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    if (jadren_app_state[slot].kind == 0) {
        current = 0.0;
    } else if (jadren_app_state[slot].kind == JADREN_APP_STATE_FLOAT) {
        current = jadren_app_state[slot].float_value;
    } else {
        return 0;
    }
    sum = current + delta;
    sum_representation.value = sum;
    if (((sum_representation.bits >> 52) & 0x7FFULL) == 0x7FFULL ||
        format_float(sum, encoded, sizeof(encoded)) == 0) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_FLOAT;
    jadren_app_state[slot].float_value = sum;
    app_state_bump_revision();
    return 1;
}

/* Revision-guarded finite floating-point delta. Stale, invalid, mismatched,
 * non-finite and overflowing calls leave both value and revision unchanged. */
int app_state_add_float_if_revision(const char *key_data,
                                    unsigned __int64 key_length,
                                    double delta,
                                    unsigned __int64 expected_revision) {
    union {
        double value;
        unsigned long long bits;
    } delta_representation, sum_representation;
    unsigned char encoded[32];
    int slot;
    double current;
    double sum;
    if (app_state_revision() != expected_revision) {
        return 0;
    }
    delta_representation.value = delta;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length) ||
        ((delta_representation.bits >> 52) & 0x7FFULL) == 0x7FFULL) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    if (jadren_app_state[slot].kind == 0) {
        current = 0.0;
    } else if (jadren_app_state[slot].kind == JADREN_APP_STATE_FLOAT) {
        current = jadren_app_state[slot].float_value;
    } else {
        return 0;
    }
    sum = current + delta;
    sum_representation.value = sum;
    if (((sum_representation.bits >> 52) & 0x7FFULL) == 0x7FFULL ||
        format_float(sum, encoded, sizeof(encoded)) == 0) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_FLOAT;
    jadren_app_state[slot].float_value = sum;
    app_state_bump_revision();
    return 1;
}

unsigned __int64 app_state_get_uint(const char *key_data, unsigned __int64 key_length) {
    int slot = app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    if (jadren_app_state[slot].kind == JADREN_APP_STATE_UINT) {
        return jadren_app_state[slot].uint_value;
    }
    if (jadren_app_state[slot].kind == JADREN_APP_STATE_INT &&
        jadren_app_state[slot].int_value >= 0) {
        return (unsigned __int64)jadren_app_state[slot].int_value;
    }
    return 0;
}

int app_state_set_float(const char *key_data, unsigned __int64 key_length,
                        double value) {
    union {
        double value;
        unsigned long long bits;
    } representation;
    unsigned char encoded[32];
    int slot;
    representation.value = value;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length) ||
        ((representation.bits >> 52) & 0x7FFULL) == 0x7FFULL ||
        format_float(value, encoded, sizeof(encoded)) == 0) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_FLOAT;
    jadren_app_state[slot].float_value = value;
    app_state_bump_revision();
    return 1;
}

/* Replace a finite Float64 value only when the caller's equality-only
 * revision is current. Non-finite and stale calls leave state unchanged. */
int app_state_set_float_if_revision(const char *key_data,
                                    unsigned __int64 key_length,
                                    double value,
                                    unsigned __int64 expected_revision) {
    union {
        double value;
        unsigned long long bits;
    } representation;
    unsigned char encoded[32];
    int slot;
    if (app_state_revision() != expected_revision) return 0;
    representation.value = value;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length) ||
        ((representation.bits >> 52) & 0x7FFULL) == 0x7FFULL ||
        format_float(value, encoded, sizeof(encoded)) == 0) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) return 0;
    jadren_app_state[slot].kind = JADREN_APP_STATE_FLOAT;
    jadren_app_state[slot].float_value = value;
    app_state_bump_revision();
    return 1;
}

double app_state_get_float(const char *key_data, unsigned __int64 key_length) {
    int slot = app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0.0;
    }
    if (jadren_app_state[slot].kind == JADREN_APP_STATE_FLOAT) {
        return jadren_app_state[slot].float_value;
    }
    if (jadren_app_state[slot].kind == JADREN_APP_STATE_INT) {
        return (double)jadren_app_state[slot].int_value;
    }
    if (jadren_app_state[slot].kind == JADREN_APP_STATE_UINT) {
        return (double)jadren_app_state[slot].uint_value;
    }
    return 0.0;
}

int app_state_set_bool(const char *key_data, unsigned __int64 key_length,
                       unsigned char value) {
    int slot;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_BOOL;
    jadren_app_state[slot].bool_value = value != 0;
    app_state_bump_revision();
    return 1;
}

/* Replace a Bool only when the caller's equality-only revision is current. */
int app_state_set_bool_if_revision(const char *key_data,
                                   unsigned __int64 key_length,
                                   unsigned char value,
                                   unsigned __int64 expected_revision) {
    int slot;
    if (app_state_revision() != expected_revision) return 0;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length)) return 0;
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) return 0;
    jadren_app_state[slot].kind = JADREN_APP_STATE_BOOL;
    jadren_app_state[slot].bool_value = value != 0;
    app_state_bump_revision();
    return 1;
}

int app_state_get_bool(const char *key_data, unsigned __int64 key_length) {
    int slot = app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    return slot >= 0 && jadren_app_state[slot].kind == JADREN_APP_STATE_BOOL &&
           jadren_app_state[slot].bool_value != 0;
}

int app_state_set_text(const char *key_data, unsigned __int64 key_length,
                       const char *value_data, unsigned __int64 value_length) {
    int slot;
    unsigned __int64 index;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length) ||
        (value_data == 0 && value_length > 0) ||
        value_length > JADREN_APP_STATE_TEXT_MAX) {
        return 0;
    }
    slot = app_state_slot_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    jadren_app_state[slot].kind = JADREN_APP_STATE_TEXT;
    jadren_app_state[slot].text_length = value_length;
    for (index = 0; index < value_length; index += 1) {
        jadren_app_state[slot].text[index] = (unsigned char)value_data[index];
    }
    app_state_bump_revision();
    return 1;
}

/* Replace bounded UTF-8 text only when the caller's equality-only revision is
 * current. Validate the full input before allocating a state slot or copying. */
int app_state_set_text_if_revision(const char *key_data,
                                   unsigned __int64 key_length,
                                   const char *value_data,
                                   unsigned __int64 value_length,
                                   unsigned __int64 expected_revision) {
    int slot;
    unsigned __int64 index;
    if (app_state_revision() != expected_revision) return 0;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length) ||
        (value_data == 0 && value_length > 0) ||
        value_length > JADREN_APP_STATE_TEXT_MAX) return 0;
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) return 0;
    jadren_app_state[slot].kind = JADREN_APP_STATE_TEXT;
    jadren_app_state[slot].text_length = value_length;
    for (index = 0; index < value_length; index += 1) {
        jadren_app_state[slot].text[index] = (unsigned char)value_data[index];
    }
    app_state_bump_revision();
    return 1;
}

int app_state_set_text_bytes(const char *key_data, unsigned __int64 key_length,
                             const unsigned char *value_data,
                             unsigned __int64 value_capacity,
                             unsigned __int64 value_length) {
    if (value_data == 0 && value_length > 0) return 0;
    if (value_length > value_capacity) value_length = value_capacity;
    return app_state_set_text(key_data, key_length, (const char *)value_data,
                              value_length);
}

/* Revision-guarded caller-owned UTF-8 input. The explicit length must fit the
 * source slice and the bounded state slot; unlike the legacy bytes setter,
 * this path rejects an overlong prefix instead of silently truncating it. */
int app_state_set_text_bytes_if_revision(
    const char *key_data, unsigned __int64 key_length,
    const unsigned char *value_data, unsigned __int64 value_capacity,
    unsigned __int64 value_length, unsigned __int64 expected_revision) {
    int slot;
    unsigned __int64 index;
    if (app_state_revision() != expected_revision) return 0;
    if (!app_state_key_is_safe((const unsigned char *)key_data, key_length) ||
        (value_data == 0 && value_length > 0) ||
        value_length > value_capacity ||
        value_length > JADREN_APP_STATE_TEXT_MAX) return 0;
    slot = app_state_slot_in(jadren_app_state,
                             (const unsigned char *)key_data, key_length);
    if (slot < 0) return 0;
    jadren_app_state[slot].kind = JADREN_APP_STATE_TEXT;
    jadren_app_state[slot].text_length = value_length;
    for (index = 0; index < value_length; index += 1) {
        jadren_app_state[slot].text[index] = value_data[index];
    }
    app_state_bump_revision();
    return 1;
}

unsigned __int64 app_state_read_text(const char *key_data, unsigned __int64 key_length,
                                     unsigned char *output_data,
                                     unsigned __int64 output_length) {
    int slot = app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    unsigned __int64 index;
    if (slot < 0 || jadren_app_state[slot].kind != JADREN_APP_STATE_TEXT ||
        jadren_app_state[slot].text_length == 0 || output_data == 0 ||
        output_length < jadren_app_state[slot].text_length) {
        return 0;
    }
    for (index = 0; index < jadren_app_state[slot].text_length; index += 1) {
        output_data[index] = jadren_app_state[slot].text[index];
    }
    return jadren_app_state[slot].text_length;
}

int app_state_read_text_exact(const char *key_data, unsigned __int64 key_length,
                              unsigned char *output_data, unsigned __int64 output_length,
                              unsigned __int64 *output_text_length,
                              unsigned __int64 output_text_length_capacity) {
    int slot = app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    unsigned __int64 index;
    unsigned __int64 text_length;
    if (slot < 0 || jadren_app_state[slot].kind != JADREN_APP_STATE_TEXT ||
        output_text_length == 0 || output_text_length_capacity == 0 ||
        (output_data == 0 && jadren_app_state[slot].text_length > 0) ||
        output_length < jadren_app_state[slot].text_length) {
        return 0;
    }
    text_length = jadren_app_state[slot].text_length;
    for (index = 0; index < text_length; index += 1) {
        output_data[index] = jadren_app_state[slot].text[index];
    }
    output_text_length[0] = text_length;
    return 1;
}

int app_state_read_int(const char *key_data, unsigned __int64 key_length,
                       long long *output_data, unsigned __int64 output_length) {
    int slot = app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0 || jadren_app_state[slot].kind != JADREN_APP_STATE_INT ||
        output_data == 0 || output_length == 0) return 0;
    output_data[0] = jadren_app_state[slot].int_value;
    return 1;
}

int app_state_read_uint(const char *key_data, unsigned __int64 key_length,
                        unsigned __int64 *output_data, unsigned __int64 output_length) {
    int slot = app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0 || jadren_app_state[slot].kind != JADREN_APP_STATE_UINT ||
        output_data == 0 || output_length == 0) return 0;
    output_data[0] = jadren_app_state[slot].uint_value;
    return 1;
}

int app_state_read_float(const char *key_data, unsigned __int64 key_length,
                         double *output_data, unsigned __int64 output_length) {
    int slot = app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0 || jadren_app_state[slot].kind != JADREN_APP_STATE_FLOAT ||
        output_data == 0 || output_length == 0) return 0;
    output_data[0] = jadren_app_state[slot].float_value;
    return 1;
}

int app_state_read_bool(const char *key_data, unsigned __int64 key_length,
                        unsigned char *output_data, unsigned __int64 output_length) {
    int slot = app_state_find_in(jadren_app_state, (const unsigned char *)key_data, key_length);
    if (slot < 0 || jadren_app_state[slot].kind != JADREN_APP_STATE_BOOL ||
        output_data == 0 || output_length == 0) return 0;
    output_data[0] = jadren_app_state[slot].bool_value;
    return 1;
}

static int app_state_append_bytes(unsigned char *output_data,
                                  unsigned __int64 output_length,
                                  unsigned __int64 *offset,
                                  const unsigned char *input_data,
                                  unsigned __int64 input_length) {
    unsigned __int64 index;
    if (*offset > output_length || input_length > output_length - *offset ||
        (input_data == 0 && input_length > 0)) {
        return 0;
    }
    for (index = 0; index < input_length; index += 1) {
        output_data[*offset + index] = input_data[index];
    }
    *offset += input_length;
    return 1;
}

static int app_state_append_byte(unsigned char *output_data,
                                 unsigned __int64 output_length,
                                 unsigned __int64 *offset,
                                 unsigned char value) {
    return app_state_append_bytes(output_data, output_length, offset, &value, 1);
}

static int app_state_append_json_string(unsigned char *output_data,
                                        unsigned __int64 output_length,
                                        unsigned __int64 *offset,
                                        const unsigned char *value_data,
                                        unsigned __int64 value_length) {
    unsigned __int64 index;
    if (!app_state_append_byte(output_data, output_length, offset, (unsigned char)'"')) {
        return 0;
    }
    for (index = 0; index < value_length; index += 1) {
        unsigned char value = value_data[index];
        if (value == (unsigned char)'"' || value == (unsigned char)'\\') {
            if (!app_state_append_byte(output_data, output_length, offset, (unsigned char)'\\') ||
                !app_state_append_byte(output_data, output_length, offset, value)) {
                return 0;
            }
        } else if (value == '\n' || value == '\r' || value == '\t' || value == '\b' || value == '\f') {
            unsigned char escape = value == '\n' ? 'n' : value == '\r' ? 'r' :
                                   value == '\t' ? 't' : value == '\b' ? 'b' : 'f';
            if (!app_state_append_byte(output_data, output_length, offset, (unsigned char)'\\') ||
                !app_state_append_byte(output_data, output_length, offset, escape)) {
                return 0;
            }
        } else if (value < 0x20U) {
            return 0;
        } else if (!app_state_append_byte(output_data, output_length, offset, value)) {
            return 0;
        }
    }
    return app_state_append_byte(output_data, output_length, offset, (unsigned char)'"');
}

static int app_state_append_number(unsigned char *output_data,
                                   unsigned __int64 output_length,
                                   unsigned __int64 *offset,
                                   const unsigned char *number_data,
                                   unsigned __int64 number_length) {
    return app_state_append_bytes(output_data, output_length, offset,
                                  number_data, number_length);
}

int app_state_save(const char *path_data, unsigned __int64 path_length) {
    unsigned char document[JADREN_APP_STATE_DOCUMENT_MAX];
    unsigned char number_data[32];
    unsigned __int64 offset = 0;
    unsigned int entry_index;
    int first = 1;
    if (!app_state_append_byte(document, sizeof(document), &offset, (unsigned char)'{')) {
        return 0;
    }
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        JadrenAppStateEntry *entry = &jadren_app_state[entry_index];
        unsigned __int64 number_length;
        if (!entry->used || entry->kind == 0) {
            continue;
        }
        if (!first && !app_state_append_byte(document, sizeof(document), &offset, (unsigned char)',')) {
            return 0;
        }
        first = 0;
        if (!app_state_append_json_string(document, sizeof(document), &offset,
                                          entry->key, entry->key_length) ||
            !app_state_append_byte(document, sizeof(document), &offset, (unsigned char)':')) {
            return 0;
        }
        if (entry->kind == JADREN_APP_STATE_TEXT) {
            if (!app_state_append_json_string(document, sizeof(document), &offset,
                                              entry->text, entry->text_length)) {
                return 0;
            }
        } else if (entry->kind == JADREN_APP_STATE_BOOL) {
            const unsigned char *value = entry->bool_value ?
                (const unsigned char *)"true" : (const unsigned char *)"false";
            if (!app_state_append_bytes(document, sizeof(document), &offset,
                                        value, entry->bool_value ? 4 : 5)) {
                return 0;
            }
        } else if (entry->kind == JADREN_APP_STATE_INT) {
            number_length = format_int(entry->int_value, number_data, sizeof(number_data));
            if (number_length == 0 || !app_state_append_number(document, sizeof(document),
                                                                &offset, number_data, number_length)) {
                return 0;
            }
        } else if (entry->kind == JADREN_APP_STATE_UINT) {
            number_length = format_uint(entry->uint_value, number_data, sizeof(number_data));
            if (number_length == 0 || !app_state_append_number(document, sizeof(document),
                                                                &offset, number_data, number_length)) {
                return 0;
            }
        } else if (entry->kind == JADREN_APP_STATE_FLOAT) {
            number_length = format_float(entry->float_value, number_data, sizeof(number_data));
            if (number_length == 0 || !app_state_append_number(document, sizeof(document),
                                                                &offset, number_data, number_length)) {
                return 0;
            }
        } else {
            return 0;
        }
    }
    if (!app_state_append_byte(document, sizeof(document), &offset, (unsigned char)'}')) {
        return 0;
    }
    return file_write_text(path_data, path_length, (const char *)document, offset) == offset;
}

/* Export a complete snapshot without forcing callers through the filesystem.
 * Build into the bounded native document first so a short caller buffer never
 * receives a partial JSON value or a length update. */
static int app_state_parse_document(const unsigned char *data,
                                    unsigned __int64 length,
                                    JadrenAppStateEntry *entries);
int app_state_write_json_exact(unsigned char *output_data,
                               unsigned __int64 output_length,
                               unsigned __int64 *written_data,
                               unsigned __int64 written_capacity) {
    unsigned char document[JADREN_APP_STATE_DOCUMENT_MAX];
    unsigned char number_data[32];
    unsigned __int64 offset = 0;
    unsigned int entry_index;
    int first = 1;
    if (output_data == 0 || output_length == 0 || written_data == 0 ||
        written_capacity == 0 ||
        !app_state_append_byte(document, sizeof(document), &offset, (unsigned char)'{')) {
        return 0;
    }
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        JadrenAppStateEntry *entry = &jadren_app_state[entry_index];
        unsigned __int64 number_length;
        if (!entry->used || entry->kind == 0) continue;
        if (!first && !app_state_append_byte(document, sizeof(document), &offset, (unsigned char)',')) return 0;
        first = 0;
        if (!app_state_append_json_string(document, sizeof(document), &offset,
                                          entry->key, entry->key_length) ||
            !app_state_append_byte(document, sizeof(document), &offset, (unsigned char)':')) return 0;
        if (entry->kind == JADREN_APP_STATE_TEXT) {
            if (!app_state_append_json_string(document, sizeof(document), &offset,
                                              entry->text, entry->text_length)) return 0;
        } else if (entry->kind == JADREN_APP_STATE_BOOL) {
            const unsigned char *value = entry->bool_value ?
                (const unsigned char *)"true" : (const unsigned char *)"false";
            if (!app_state_append_bytes(document, sizeof(document), &offset,
                                        value, entry->bool_value ? 4 : 5)) return 0;
        } else if (entry->kind == JADREN_APP_STATE_INT) {
            number_length = format_int(entry->int_value, number_data, sizeof(number_data));
            if (number_length == 0 || !app_state_append_number(document, sizeof(document),
                                                                &offset, number_data, number_length)) return 0;
        } else if (entry->kind == JADREN_APP_STATE_UINT) {
            number_length = format_uint(entry->uint_value, number_data, sizeof(number_data));
            if (number_length == 0 || !app_state_append_number(document, sizeof(document),
                                                                &offset, number_data, number_length)) return 0;
        } else if (entry->kind == JADREN_APP_STATE_FLOAT) {
            number_length = format_float(entry->float_value, number_data, sizeof(number_data));
            if (number_length == 0 || !app_state_append_number(document, sizeof(document),
                                                                &offset, number_data, number_length)) return 0;
        } else {
            return 0;
        }
    }
    if (!app_state_append_byte(document, sizeof(document), &offset, (unsigned char)'}') ||
        offset > output_length) return 0;
    for (entry_index = 0; entry_index < offset; entry_index += 1) output_data[entry_index] = document[entry_index];
    written_data[0] = offset;
    return 1;
}

/* Parse one caller-owned JSON prefix into a temporary state first. The live
 * store changes only after the complete document has passed validation. */
int app_state_load_json_exact(const unsigned char *input_data,
                              unsigned __int64 input_capacity,
                              unsigned __int64 input_length) {
    JadrenAppStateEntry parsed[JADREN_APP_STATE_MAX_ENTRIES];
    unsigned int entry_index;
    unsigned __int64 byte_index;
    if (input_data == 0 || input_length == 0 || input_length > input_capacity ||
        input_length > JADREN_APP_STATE_DOCUMENT_MAX) return 0;
    app_state_clear_entries(parsed);
    if (!app_state_parse_document(input_data, input_length, parsed)) return 0;
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        jadren_app_state[entry_index].used = parsed[entry_index].used;
        jadren_app_state[entry_index].kind = parsed[entry_index].kind;
        jadren_app_state[entry_index].key_length = parsed[entry_index].key_length;
        jadren_app_state[entry_index].int_value = parsed[entry_index].int_value;
        jadren_app_state[entry_index].uint_value = parsed[entry_index].uint_value;
        jadren_app_state[entry_index].bool_value = parsed[entry_index].bool_value;
        jadren_app_state[entry_index].float_value = parsed[entry_index].float_value;
        jadren_app_state[entry_index].text_length = parsed[entry_index].text_length;
        for (byte_index = 0; byte_index < JADREN_APP_STATE_KEY_MAX; byte_index += 1)
            jadren_app_state[entry_index].key[byte_index] = parsed[entry_index].key[byte_index];
        for (byte_index = 0; byte_index < JADREN_APP_STATE_TEXT_MAX; byte_index += 1)
            jadren_app_state[entry_index].text[byte_index] = parsed[entry_index].text[byte_index];
    }
    app_state_bump_revision();
    app_data_refresh_bound_ui();
    return 1;
}

int file_replace_atomic(const char *source_data, unsigned __int64 source_length,
                        const char *target_data, unsigned __int64 target_length);
int file_flush(const char *path_data, unsigned __int64 path_length);
int directory_flush(const char *path_data, unsigned __int64 path_length);
int directory_exists(const char *path_data, unsigned __int64 path_length);

int app_state_save_atomic(const char *temporary_path_data,
                          unsigned __int64 temporary_path_length,
                          const char *target_path_data,
                          unsigned __int64 target_path_length) {
    if (temporary_path_data == 0 || target_path_data == 0 ||
        temporary_path_length == 0 || target_path_length == 0 ||
        !app_state_save(temporary_path_data, temporary_path_length)) {
        return 0;
    }
    return file_replace_atomic(temporary_path_data, temporary_path_length,
                                target_path_data, target_path_length);
}

int app_state_save_atomic_if_revision(const char *temporary_path_data,
                                      unsigned __int64 temporary_path_length,
                                      const char *target_path_data,
                                      unsigned __int64 target_path_length,
                                      unsigned __int64 expected_revision) {
    if (jadren_app_state_revision != expected_revision) {
        return 0;
    }
    return app_state_save_atomic(temporary_path_data, temporary_path_length,
                                 target_path_data, target_path_length);
}

/* Durable app-state checkpoint with explicit directory metadata flush. The
 * caller must reload/verify after a post-promotion failure before retrying. */
int app_state_save_atomic_durable(const char *temporary_path_data,
                                  unsigned __int64 temporary_path_length,
                                  const char *target_path_data,
                                  unsigned __int64 target_path_length,
                                  const char *directory_path_data,
                                  unsigned __int64 directory_path_length) {
    unsigned __int64 path_index;
    int same_path = temporary_path_length == target_path_length;
    if (same_path) {
        for (path_index = 0; path_index < temporary_path_length; path_index += 1) {
            if (temporary_path_data == 0 || target_path_data == 0 ||
                temporary_path_data[path_index] != target_path_data[path_index]) {
                same_path = 0;
                break;
            }
        }
    }
    if (same_path || temporary_path_data == 0 || target_path_data == 0 ||
        directory_path_data == 0 || temporary_path_length == 0 ||
        target_path_length == 0 || directory_path_length == 0 ||
        !directory_exists(directory_path_data, directory_path_length) ||
        !app_state_save(temporary_path_data, temporary_path_length) ||
        !file_flush(temporary_path_data, temporary_path_length) ||
        !file_replace_atomic(temporary_path_data, temporary_path_length,
                             target_path_data, target_path_length)) {
        return 0;
    }
    if (!file_flush(target_path_data, target_path_length)) return 0;
    return directory_flush(directory_path_data, directory_path_length);
}

int app_state_save_atomic_durable_if_revision(
    const char *temporary_path_data,
    unsigned __int64 temporary_path_length,
    const char *target_path_data,
    unsigned __int64 target_path_length,
    const char *directory_path_data,
    unsigned __int64 directory_path_length,
    unsigned __int64 expected_revision) {
    if (jadren_app_state_revision != expected_revision) return 0;
    return app_state_save_atomic_durable(
        temporary_path_data, temporary_path_length, target_path_data,
        target_path_length, directory_path_data, directory_path_length);
}

static int app_state_parse_value(const unsigned char *data,
                                 unsigned __int64 start,
                                 unsigned __int64 end,
                                 const unsigned char *key_data,
                                 unsigned __int64 key_length,
                                 JadrenAppStateEntry *entries) {
    int slot;
    unsigned __int64 index;
    if (!app_state_key_is_safe(key_data, key_length) || start >= end) {
        return 0;
    }
    slot = app_state_slot_in(entries, key_data, key_length);
    if (slot < 0) {
        return 0;
    }
    if (data[start] == '"') {
        unsigned __int64 required = json_read_string_length(data, start, end);
        if (required > JADREN_APP_STATE_TEXT_MAX) {
            return 0;
        }
        entries[slot].kind = JADREN_APP_STATE_TEXT;
        entries[slot].text_length = required;
        (void)json_read_string_copy(data, start, end, entries[slot].text);
        return 1;
    }
    if (end - start == 4 && data[start] == 't' && data[start + 1] == 'r' &&
        data[start + 2] == 'u' && data[start + 3] == 'e') {
        entries[slot].kind = JADREN_APP_STATE_BOOL;
        entries[slot].bool_value = 1;
        return 1;
    }
    if (end - start == 5 && data[start] == 'f' && data[start + 1] == 'a' &&
        data[start + 2] == 'l' && data[start + 3] == 's' && data[start + 4] == 'e') {
        entries[slot].kind = JADREN_APP_STATE_BOOL;
        entries[slot].bool_value = 0;
        return 1;
    }
    for (index = start; index < end; index += 1) {
        if (data[index] == '.' || data[index] == 'e' || data[index] == 'E') {
            if (!json_read_float_value(data, start, end, &entries[slot].float_value)) {
                return 0;
            }
            entries[slot].kind = JADREN_APP_STATE_FLOAT;
            return 1;
        }
    }
    if (data[start] == '-') {
        if (!json_read_int_value(data, start, end, &entries[slot].int_value)) {
            return 0;
        }
        entries[slot].kind = JADREN_APP_STATE_INT;
        return 1;
    }
    if (!json_read_uint_value(data, start, end, &entries[slot].uint_value)) {
        return 0;
    }
    entries[slot].kind = JADREN_APP_STATE_UINT;
    return 1;
}

static int app_state_parse_document(const unsigned char *data,
                                    unsigned __int64 length,
                                    JadrenAppStateEntry *entries) {
    unsigned __int64 index = 0;
    unsigned __int64 key_start;
    unsigned __int64 key_end;
    unsigned __int64 value_start;
    unsigned __int64 value_end;
    unsigned __int64 key_length;
    if (data == 0 || length == 0 || !json_read_skip_ws(data, length, &index) ||
        data[index] != '{') {
        return 0;
    }
    index += 1;
    while (1) {
        while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                  data[index] == '\r' || data[index] == '\n')) {
            index += 1;
        }
        if (index >= length) {
            return 0;
        }
        if (data[index] == '}') {
            index += 1;
            while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                      data[index] == '\r' || data[index] == '\n')) {
                index += 1;
            }
            return index == length;
        }
        key_start = index;
        if (!json_read_string_end(data, length, key_start, &key_end) ||
            key_end <= key_start + 2) {
            return 0;
        }
        key_length = key_end - key_start - 2;
        if (key_length > JADREN_APP_STATE_KEY_MAX ||
            !app_state_key_is_safe(data + key_start + 1, key_length)) {
            return 0;
        }
        index = key_end;
        while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                  data[index] == '\r' || data[index] == '\n')) {
            index += 1;
        }
        if (index >= length || data[index] != ':') {
            return 0;
        }
        index += 1;
        while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                  data[index] == '\r' || data[index] == '\n')) {
            index += 1;
        }
        if (index >= length) {
            return 0;
        }
        value_start = index;
        if (data[index] == '"') {
            if (!json_read_string_end(data, length, index, &value_end)) {
                return 0;
            }
            index = value_end;
        } else {
            if (data[index] == '{' || data[index] == '[') {
                return 0;
            }
            while (index < length && data[index] != ',' && data[index] != '}') {
                index += 1;
            }
            value_end = index;
            while (value_end > value_start && (data[value_end - 1] == ' ' ||
                                               data[value_end - 1] == '\t' ||
                                               data[value_end - 1] == '\r' ||
                                               data[value_end - 1] == '\n')) {
                value_end -= 1;
            }
            if (value_end == value_start) {
                return 0;
            }
        }
        if (!app_state_parse_value(data, value_start, value_end,
                                   data + key_start + 1, key_length, entries)) {
            return 0;
        }
        while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                  data[index] == '\r' || data[index] == '\n')) {
            index += 1;
        }
        if (index >= length) {
            return 0;
        }
        if (data[index] == ',') {
            index += 1;
            continue;
        }
        if (data[index] != '}') {
            return 0;
        }
        index += 1;
        while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                  data[index] == '\r' || data[index] == '\n')) {
            index += 1;
        }
        return index == length;
    }
}

int app_state_load(const char *path_data, unsigned __int64 path_length) {
    unsigned char document[JADREN_APP_STATE_DOCUMENT_MAX];
    JadrenAppStateEntry parsed[JADREN_APP_STATE_MAX_ENTRIES];
    unsigned int entry_index;
    unsigned __int64 byte_index;
    unsigned __int64 document_length;
    if (file_size(path_data, path_length) > JADREN_APP_STATE_DOCUMENT_MAX) {
        return 0;
    }
    document_length = file_read(path_data, path_length, document, sizeof(document));
    app_state_clear_entries(parsed);
    if (!app_state_parse_document(document, document_length, parsed)) {
        return 0;
    }
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        jadren_app_state[entry_index].used = parsed[entry_index].used;
        jadren_app_state[entry_index].kind = parsed[entry_index].kind;
        jadren_app_state[entry_index].key_length = parsed[entry_index].key_length;
        jadren_app_state[entry_index].int_value = parsed[entry_index].int_value;
        jadren_app_state[entry_index].uint_value = parsed[entry_index].uint_value;
        jadren_app_state[entry_index].bool_value = parsed[entry_index].bool_value;
        jadren_app_state[entry_index].float_value = parsed[entry_index].float_value;
        jadren_app_state[entry_index].text_length = parsed[entry_index].text_length;
        for (byte_index = 0; byte_index < JADREN_APP_STATE_KEY_MAX; byte_index += 1) {
            jadren_app_state[entry_index].key[byte_index] = parsed[entry_index].key[byte_index];
        }
        for (byte_index = 0; byte_index < JADREN_APP_STATE_TEXT_MAX; byte_index += 1) {
            jadren_app_state[entry_index].text[byte_index] = parsed[entry_index].text[byte_index];
        }
    }
    app_state_bump_revision();
    app_data_refresh_bound_ui();
    return 1;
}

#define JADREN_APP_LIST_MAX_LISTS 4
#define JADREN_APP_LIST_MAX_ITEMS 64
#define JADREN_APP_LIST_TEXT_MAX 256

typedef struct JadrenAppListItem {
    unsigned __int64 length;
    unsigned char text[JADREN_APP_LIST_TEXT_MAX];
} JadrenAppListItem;

typedef struct JadrenAppList {
    unsigned __int64 count;
    JadrenAppListItem items[JADREN_APP_LIST_MAX_ITEMS];
} JadrenAppList;

static JadrenAppList jadren_app_lists[JADREN_APP_LIST_MAX_LISTS];

static void app_list_copy_store(JadrenAppList *destination, const JadrenAppList *source) {
    unsigned int index;
    unsigned __int64 byte_index;
    if (destination == 0 || source == 0) return;
    destination->count = source->count;
    for (index = 0; index < JADREN_APP_LIST_MAX_ITEMS; index += 1) {
        destination->items[index].length = source->items[index].length;
        for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1)
            destination->items[index].text[byte_index] = source->items[index].text[byte_index];
    }
}

static unsigned int app_list_fingerprint(const JadrenAppList *list) {
    unsigned int hash = 2166136261U;
    unsigned int item_index;
    if (list == 0) return 0;
    hash ^= (unsigned int)list->count;
    hash *= 16777619U;
    hash ^= (unsigned int)(list->count >> 32);
    hash *= 16777619U;
    for (item_index = 0; item_index < list->count; item_index += 1) {
        const JadrenAppListItem *item = &list->items[item_index];
        unsigned __int64 byte_index;
        hash ^= (unsigned int)item->length;
        hash *= 16777619U;
        hash ^= (unsigned int)(item->length >> 32);
        hash *= 16777619U;
        for (byte_index = 0; byte_index < item->length; byte_index += 1) {
            hash ^= item->text[byte_index];
            hash *= 16777619U;
        }
    }
    return hash;
}

static int app_list_valid(int list_id) {
    return list_id >= 0 && list_id < JADREN_APP_LIST_MAX_LISTS;
}

static void app_list_clear_item(JadrenAppListItem *item) {
    unsigned __int64 index;
    item->length = 0;
    for (index = 0; index < JADREN_APP_LIST_TEXT_MAX; index += 1) {
        item->text[index] = 0;
    }
}

static void app_list_clear_store(JadrenAppList *list) {
    unsigned int index;
    if (list == 0) {
        return;
    }
    list->count = 0;
    for (index = 0; index < JADREN_APP_LIST_MAX_ITEMS; index += 1) {
        app_list_clear_item(&list->items[index]);
    }
}

void app_list_clear(int list_id) {
    if (!app_list_valid(list_id)) {
        return;
    }
    app_list_clear_store(&jadren_app_lists[list_id]);
}

int app_list_count(int list_id) {
    if (!app_list_valid(list_id)) {
        return 0;
    }
    return (int)jadren_app_lists[list_id].count;
}

static int app_list_compare_items(const JadrenAppListItem *left,
                                  const JadrenAppListItem *right) {
    unsigned __int64 index;
    unsigned __int64 common_length = left->length < right->length ? left->length : right->length;
    for (index = 0; index < common_length; index += 1) {
        if (left->text[index] < right->text[index]) return -1;
        if (left->text[index] > right->text[index]) return 1;
    }
    if (left->length < right->length) return -1;
    if (left->length > right->length) return 1;
    return 0;
}

static void app_list_swap_items(JadrenAppListItem *left, JadrenAppListItem *right) {
    unsigned __int64 byte_index;
    unsigned __int64 length = left->length;
    left->length = right->length;
    right->length = length;
    for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1) {
        unsigned char value = left->text[byte_index];
        left->text[byte_index] = right->text[byte_index];
        right->text[byte_index] = value;
    }
}

int app_list_sort_text(int list_id, int descending) {
    unsigned __int64 row_index;
    if (!app_list_valid(list_id)) return 0;
    for (row_index = 1; row_index < jadren_app_lists[list_id].count; row_index += 1) {
        unsigned __int64 cursor = row_index;
        while (cursor > 0) {
            JadrenAppListItem *previous = &jadren_app_lists[list_id].items[cursor - 1];
            JadrenAppListItem *current = &jadren_app_lists[list_id].items[cursor];
            int comparison = app_list_compare_items(previous, current);
            if (descending) comparison = -comparison;
            if (comparison <= 0) break;
            app_list_swap_items(previous, current);
            cursor -= 1;
        }
    }
    return 1;
}

int app_list_sort_callback(int list_id, int (*comparator)(int, int, int)) {
    JadrenAppList sorted;
    unsigned char items[JADREN_APP_LIST_MAX_ITEMS];
    unsigned int item_count;
    unsigned int item_index;
    unsigned int source_fingerprint;
    if (!app_list_valid(list_id) || comparator == 0) return 0;
    item_count = (unsigned int)jadren_app_lists[list_id].count;
    source_fingerprint = app_list_fingerprint(&jadren_app_lists[list_id]);
    app_list_copy_store(&sorted, &jadren_app_lists[list_id]);
    for (item_index = 0; item_index < item_count; item_index += 1)
        items[item_index] = (unsigned char)item_index;
    for (item_index = 1; item_index < item_count; item_index += 1) {
        unsigned char current = items[item_index];
        unsigned int cursor = item_index;
        while (cursor > 0) {
            unsigned char previous = items[cursor - 1];
            int comparison;
            if (app_list_fingerprint(&jadren_app_lists[list_id]) != source_fingerprint)
                return 0;
            comparison = comparator(list_id, (int)previous, (int)current);
            if (app_list_fingerprint(&jadren_app_lists[list_id]) != source_fingerprint)
                return 0;
            if (comparison <= 0) break;
            items[cursor] = previous;
            cursor -= 1;
        }
        items[cursor] = current;
    }
    for (item_index = 0; item_index < item_count; item_index += 1) {
        unsigned char source_item = items[item_index];
        unsigned __int64 byte_index;
        sorted.items[item_index].length =
            jadren_app_lists[list_id].items[source_item].length;
        for (byte_index = 0; byte_index < sorted.items[item_index].length;
             byte_index += 1)
            sorted.items[item_index].text[byte_index] =
                jadren_app_lists[list_id].items[source_item].text[byte_index];
    }
    if (app_list_fingerprint(&jadren_app_lists[list_id]) != source_fingerprint) return 0;
    app_list_copy_store(&jadren_app_lists[list_id], &sorted);
    return 1;
}

/* Sort through a caller-owned comparator only when the equality-only model
 * revision is still current. The unguarded path keeps its fingerprint checks
 * around every comparator call and publishes only after the complete scan. */
int app_list_sort_callback_if_revision(
    int list_id, int (*comparator)(int, int, int),
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_sort_callback(list_id, comparator);
}

int app_list_find_text(int list_id, const char *query_data,
                       unsigned __int64 query_length, int start_index) {
    unsigned __int64 item_index;
    if (!app_list_valid(list_id) || start_index < 0 ||
        (unsigned __int64)start_index > jadren_app_lists[list_id].count ||
        (query_data == 0 && query_length > 0) || query_length > JADREN_APP_LIST_TEXT_MAX) return -1;
    for (item_index = (unsigned __int64)start_index;
         item_index < jadren_app_lists[list_id].count; item_index += 1) {
        JadrenAppListItem *item = &jadren_app_lists[list_id].items[item_index];
        unsigned __int64 byte_index;
        if (item->length != query_length) continue;
        for (byte_index = 0; byte_index < query_length; byte_index += 1)
            if (item->text[byte_index] != (unsigned char)query_data[byte_index]) break;
        if (byte_index == query_length) return (int)item_index;
    }
    return -1;
}

int app_list_push_text(int list_id, const char *value_data,
                       unsigned __int64 value_length) {
    JadrenAppListItem *item;
    unsigned __int64 index;
    if (!app_list_valid(list_id) || (value_data == 0 && value_length > 0) ||
        value_length > JADREN_APP_LIST_TEXT_MAX ||
        jadren_app_lists[list_id].count >= JADREN_APP_LIST_MAX_ITEMS) {
        return 0;
    }
    item = &jadren_app_lists[list_id].items[jadren_app_lists[list_id].count];
    item->length = value_length;
    for (index = 0; index < value_length; index += 1) {
        item->text[index] = (unsigned char)value_data[index];
    }
    jadren_app_lists[list_id].count += 1;
    return 1;
}

int app_list_push_text_bytes(int list_id, const unsigned char *value_data,
                             unsigned __int64 value_capacity,
                             unsigned __int64 value_length) {
    if (value_length > value_capacity) return 0;
    return app_list_push_text(list_id, (const char *)value_data, value_length);
}

/* Append only when the caller's equality-only model revision is still
 * current. This is process-local coordination, not a cross-thread atomic or
 * persistence primitive. */
int app_list_push_text_if_revision(int list_id, const char *value_data,
                                   unsigned __int64 value_length,
                                   unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_push_text(list_id, value_data, value_length);
}

int app_list_push_text_bytes_if_revision(int list_id, const unsigned char *value_data,
                                         unsigned __int64 value_capacity,
                                         unsigned __int64 value_length,
                                         unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision || value_length > value_capacity) return 0;
    return app_list_push_text_bytes(list_id, value_data, value_capacity, value_length);
}

static unsigned char app_list_filter_fold_ascii(unsigned char value,
                                                int case_insensitive) {
    if (case_insensitive && value >= (unsigned char)'A' && value <= (unsigned char)'Z')
        return (unsigned char)(value + ((unsigned char)'a' - (unsigned char)'A'));
    return value;
}

static int app_list_filter_matches(const JadrenAppListItem *item,
                                   const char *query_data,
                                   unsigned __int64 query_length, int mode) {
    unsigned __int64 start;
    unsigned __int64 byte_index;
    int case_insensitive = mode >= 4;
    if (case_insensitive) mode -= 4;
    if (item == 0 || mode < 0 || mode > 3 || (query_data == 0 && query_length > 0)) return 0;
    if (mode == 0 && item->length != query_length) return 0;
    if (mode != 0 && query_length > item->length) return 0;
    if (mode == 0 || mode == 2) start = 0;
    else if (mode == 3) start = item->length - query_length;
    else {
        for (start = 0; start + query_length <= item->length; start += 1) {
            for (byte_index = 0; byte_index < query_length; byte_index += 1)
                if (app_list_filter_fold_ascii(item->text[start + byte_index], case_insensitive) !=
                    app_list_filter_fold_ascii((unsigned char)query_data[byte_index], case_insensitive)) break;
            if (byte_index == query_length) return 1;
        }
        return 0;
    }
    for (byte_index = 0; byte_index < query_length; byte_index += 1)
        if (app_list_filter_fold_ascii(item->text[start + byte_index], case_insensitive) !=
            app_list_filter_fold_ascii((unsigned char)query_data[byte_index], case_insensitive)) return 0;
    return 1;
}

int app_list_filter_text_ex(int source_list_id, int destination_list_id,
                            const char *query_data, unsigned __int64 query_length, int mode) {
    unsigned __int64 item_index;
    if (!app_list_valid(source_list_id) || !app_list_valid(destination_list_id) ||
        source_list_id == destination_list_id || mode < 0 || mode > 7 ||
        query_length > JADREN_APP_LIST_TEXT_MAX || (query_data == 0 && query_length > 0)) return 0;
    app_list_clear(destination_list_id);
    for (item_index = 0; item_index < jadren_app_lists[source_list_id].count; item_index += 1) {
        JadrenAppListItem *item = &jadren_app_lists[source_list_id].items[item_index];
        if (app_list_filter_matches(item, query_data, query_length, mode) &&
            !app_list_push_text(destination_list_id, (const char *)item->text, item->length)) return 0;
    }
    return 1;
}

int app_list_filter_text(int source_list_id, int destination_list_id,
                         const char *query_data, unsigned __int64 query_length) {
    return app_list_filter_text_ex(source_list_id, destination_list_id, query_data, query_length, 0);
}

/* Filter one text list only when the caller's equality-only model revision is
 * still current. A stale revision leaves source and destination unchanged;
 * this is process-local coordination, not a cross-thread atomic or
 * persistence primitive. */
int app_list_filter_text_if_revision(int source_list_id, int destination_list_id,
                                     const char *query_data,
                                     unsigned __int64 query_length,
                                     unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_filter_text(source_list_id, destination_list_id,
                                query_data, query_length);
}

/* Filter one text list with an explicit mode only when the caller's
 * equality-only model revision is still current. A stale revision leaves
 * source and destination unchanged; this is process-local coordination, not
 * a cross-thread atomic or persistence primitive. */
int app_list_filter_text_ex_if_revision(int source_list_id, int destination_list_id,
                                        const char *query_data,
                                        unsigned __int64 query_length, int mode,
                                        unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_filter_text_ex(source_list_id, destination_list_id,
                                   query_data, query_length, mode);
}

int app_list_filter_text_ex_bytes(int source_list_id, int destination_list_id,
                                  const unsigned char *query_data,
                                  unsigned __int64 query_capacity,
                                  unsigned __int64 query_length, int mode) {
    if (query_length > query_capacity) return 0;
    return app_list_filter_text_ex(source_list_id, destination_list_id,
                                   (const char *)query_data, query_length, mode);
}

int app_list_filter_callback(int source_list_id, int destination_list_id,
                             unsigned char (*predicate)(int, int)) {
    JadrenAppList filtered;
    unsigned int source_fingerprint;
    unsigned int source_index;
    if (!app_list_valid(source_list_id) || !app_list_valid(destination_list_id) ||
        source_list_id == destination_list_id || predicate == 0) return 0;
    source_fingerprint = app_list_fingerprint(&jadren_app_lists[source_list_id]);
    app_list_copy_store(&filtered, &jadren_app_lists[source_list_id]);
    filtered.count = 0;
    for (source_index = 0; source_index < jadren_app_lists[source_list_id].count;
         source_index += 1) {
        unsigned __int64 byte_index;
        if (app_list_fingerprint(&jadren_app_lists[source_list_id]) != source_fingerprint)
            return 0;
        if (!predicate(source_list_id, (int)source_index)) continue;
        if (filtered.count >= JADREN_APP_LIST_MAX_ITEMS) return 0;
        filtered.items[filtered.count].length =
            jadren_app_lists[source_list_id].items[source_index].length;
        for (byte_index = 0; byte_index < filtered.items[filtered.count].length;
             byte_index += 1)
            filtered.items[filtered.count].text[byte_index] =
                jadren_app_lists[source_list_id].items[source_index].text[byte_index];
        filtered.count += 1;
    }
    if (app_list_fingerprint(&jadren_app_lists[source_list_id]) != source_fingerprint)
        return 0;
    app_list_copy_store(&jadren_app_lists[destination_list_id], &filtered);
    return 1;
}

/* Apply a callback filter only when the caller's equality-only model revision
 * is still current. The unguarded implementation retains its source
 * fingerprint checks while this entry point rejects a stale token before the
 * scan starts. */
int app_list_filter_callback_if_revision(
    int source_list_id, int destination_list_id,
    unsigned char (*predicate)(int, int), unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_filter_callback(source_list_id, destination_list_id, predicate);
}

int app_list_page(int source_list_id, int destination_list_id,
                  int start_index, int page_size) {
    JadrenAppList page;
    unsigned int source_count;
    unsigned int requested;
    unsigned int item_index;
    if (!app_list_valid(source_list_id) || !app_list_valid(destination_list_id) ||
        source_list_id == destination_list_id || start_index < 0 || page_size < 0 ||
        page_size > JADREN_APP_LIST_MAX_ITEMS) return 0;
    source_count = (unsigned int)jadren_app_lists[source_list_id].count;
    if ((unsigned int)start_index > source_count) return 0;
    requested = (unsigned int)page_size;
    if (requested > source_count - (unsigned int)start_index)
        requested = source_count - (unsigned int)start_index;
    app_list_copy_store(&page, &jadren_app_lists[source_list_id]);
    page.count = 0;
    for (item_index = 0; item_index < requested; item_index += 1) {
        unsigned int source_index = (unsigned int)start_index + item_index;
        unsigned __int64 byte_index;
        page.items[item_index].length = jadren_app_lists[source_list_id].items[source_index].length;
        for (byte_index = 0; byte_index < page.items[item_index].length; byte_index += 1)
            page.items[item_index].text[byte_index] =
                jadren_app_lists[source_list_id].items[source_index].text[byte_index];
    }
    page.count = requested;
    app_list_copy_store(&jadren_app_lists[destination_list_id], &page);
    return 1;
}

/* Project one bounded list page only when the caller's equality-only model
 * revision is still current. A stale revision leaves source and destination
 * lists unchanged; this is process-local coordination, not a cross-thread
 * atomic or persistence primitive. */
int app_list_page_if_revision(int source_list_id, int destination_list_id,
                              int start_index, int page_size,
                              unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_page(source_list_id, destination_list_id, start_index,
                         page_size);
}

unsigned __int64 app_list_read_text(int list_id, int item_index,
                                    unsigned char *output_data,
                                    unsigned __int64 output_length) {
    JadrenAppListItem *item;
    unsigned __int64 index;
    if (!app_list_valid(list_id) || item_index < 0 ||
        (unsigned __int64)item_index >= jadren_app_lists[list_id].count ||
        output_data == 0 ||
        output_length < jadren_app_lists[list_id].items[item_index].length) {
        return 0;
    }
    item = &jadren_app_lists[list_id].items[item_index];
    if (item->length == 0) {
        return 0;
    }
    for (index = 0; index < item->length; index += 1) {
        output_data[index] = item->text[index];
    }
    return item->length;
}

int app_list_read_text_exact(int list_id, int item_index,
                             unsigned char *output_data,
                             unsigned __int64 output_length,
                             unsigned __int64 *output_text_length,
                             unsigned __int64 output_text_length_capacity) {
    JadrenAppListItem *item;
    unsigned __int64 index;
    if (!app_list_valid(list_id) || item_index < 0 ||
        (unsigned __int64)item_index >= jadren_app_lists[list_id].count ||
        output_text_length == 0 || output_text_length_capacity == 0) {
        return 0;
    }
    item = &jadren_app_lists[list_id].items[item_index];
    if ((output_data == 0 && item->length > 0) || output_length < item->length) {
        return 0;
    }
    for (index = 0; index < item->length; index += 1) {
        output_data[index] = item->text[index];
    }
    output_text_length[0] = item->length;
    return 1;
}

int app_list_set_text(int list_id, int item_index, const char *value_data,
                      unsigned __int64 value_length) {
    JadrenAppListItem *item;
    unsigned __int64 index;
    if (!app_list_valid(list_id) || item_index < 0 ||
        (unsigned __int64)item_index >= jadren_app_lists[list_id].count ||
        (value_data == 0 && value_length > 0) ||
        value_length > JADREN_APP_LIST_TEXT_MAX) {
        return 0;
    }
    item = &jadren_app_lists[list_id].items[item_index];
    item->length = value_length;
    for (index = 0; index < value_length; index += 1) {
        item->text[index] = (unsigned char)value_data[index];
    }
    return 1;
}

/* Update one list item only when the caller's equality-only model revision is
 * still current. This is process-local coordination, not an atomic
 * cross-thread or persistence primitive. */
int app_list_set_text_if_revision(int list_id, int item_index,
                                  const char *value_data,
                                  unsigned __int64 value_length,
                                  unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_set_text(list_id, item_index, value_data, value_length);
}

int app_list_set_text_bytes(int list_id, int item_index,
                            const unsigned char *value_data,
                            unsigned __int64 value_capacity,
                            unsigned __int64 value_length) {
    if (value_length > value_capacity) return 0;
    return app_list_set_text(list_id, item_index, (const char *)value_data, value_length);
}

/* Caller-owned byte-prefix replacement guarded by the equality-only model
 * revision. Capacity is checked before the bounded setter can mutate state. */
int app_list_set_text_bytes_if_revision(int list_id, int item_index,
                                        const unsigned char *value_data,
                                        unsigned __int64 value_capacity,
                                        unsigned __int64 value_length,
                                        unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision || value_length > value_capacity) return 0;
    return app_list_set_text_bytes(list_id, item_index, value_data,
                                   value_capacity, value_length);
}

int app_list_remove(int list_id, int item_index) {
    unsigned __int64 index;
    if (!app_list_valid(list_id) || item_index < 0 ||
        (unsigned __int64)item_index >= jadren_app_lists[list_id].count) {
        return 0;
    }
    for (index = (unsigned __int64)item_index;
         index + 1 < jadren_app_lists[list_id].count; index += 1) {
        jadren_app_lists[list_id].items[index].length =
            jadren_app_lists[list_id].items[index + 1].length;
        for (unsigned __int64 byte_index = 0;
             byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1) {
            jadren_app_lists[list_id].items[index].text[byte_index] =
                jadren_app_lists[list_id].items[index + 1].text[byte_index];
        }
    }
    jadren_app_lists[list_id].count -= 1;
    app_list_clear_item(&jadren_app_lists[list_id].items[jadren_app_lists[list_id].count]);
    return 1;
}

/* Remove one list item only when the caller's equality-only model revision is
 * still current. This is process-local coordination, not a cross-thread
 * atomic or persistence primitive. */
int app_list_remove_if_revision(int list_id, int item_index,
                                unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_remove(list_id, item_index);
}

#define JADREN_APP_LIST_DOCUMENT_MAX 65536

int app_list_save(int list_id, const char *path_data, unsigned __int64 path_length) {
    unsigned char document[JADREN_APP_LIST_DOCUMENT_MAX];
    unsigned __int64 offset = 0;
    unsigned __int64 index;
    if (!app_list_valid(list_id) || path_data == 0) {
        return 0;
    }
    if (!app_state_append_byte(document, sizeof(document), &offset, (unsigned char)'[')) {
        return 0;
    }
    for (index = 0; index < jadren_app_lists[list_id].count; index += 1) {
        JadrenAppListItem *item = &jadren_app_lists[list_id].items[index];
        if (index > 0 && !app_state_append_byte(document, sizeof(document), &offset,
                                                 (unsigned char)',')) {
            return 0;
        }
        if (!app_state_append_json_string(document, sizeof(document), &offset,
                                          item->text, item->length)) {
            return 0;
        }
    }
    if (!app_state_append_byte(document, sizeof(document), &offset, (unsigned char)']')) {
        return 0;
    }
    return file_write_text(path_data, path_length, (const char *)document, offset) == offset;
}

int file_replace_atomic(const char *source_data, unsigned __int64 source_length,
                        const char *target_data, unsigned __int64 target_length);

int app_list_save_atomic(int list_id,
                         const char *temporary_path_data,
                         unsigned __int64 temporary_path_length,
                         const char *target_path_data,
                         unsigned __int64 target_path_length) {
    if (!app_list_valid(list_id) || temporary_path_data == 0 || target_path_data == 0 ||
        temporary_path_length == 0 || target_path_length == 0 ||
        !app_list_save(list_id, temporary_path_data, temporary_path_length)) {
        return 0;
    }
    return file_replace_atomic(temporary_path_data, temporary_path_length,
                                target_path_data, target_path_length);
}

static int app_list_parse_document(const unsigned char *data,
                                   unsigned __int64 length,
                                   JadrenAppList *list) {
    unsigned __int64 index = 0;
    unsigned __int64 end;
    unsigned __int64 value_length;
    JadrenAppListItem *item;
    if (data == 0 || list == 0 || length == 0 ||
        !json_read_skip_ws(data, length, &index) || data[index] != '[') {
        return 0;
    }
    index += 1;
    app_list_clear_store(list);
    for (;;) {
        while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                  data[index] == '\r' || data[index] == '\n')) {
            index += 1;
        }
        if (index >= length) {
            return 0;
        }
        if (data[index] == ']') {
            index += 1;
            while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                      data[index] == '\r' || data[index] == '\n')) {
                index += 1;
            }
            return index == length;
        }
        if (list->count >= JADREN_APP_LIST_MAX_ITEMS ||
            !json_read_string_end(data, length, index, &end)) {
            return 0;
        }
        value_length = json_read_string_length(data, index, end);
        if (value_length > JADREN_APP_LIST_TEXT_MAX) {
            return 0;
        }
        item = &list->items[list->count];
        item->length = value_length;
        (void)json_read_string_copy(data, index, end, item->text);
        list->count += 1;
        index = end;
        while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                  data[index] == '\r' || data[index] == '\n')) {
            index += 1;
        }
        if (index >= length) {
            return 0;
        }
        if (data[index] == ',') {
            index += 1;
            continue;
        }
        if (data[index] == ']') {
            index += 1;
            while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                      data[index] == '\r' || data[index] == '\n')) {
                index += 1;
            }
            return index == length;
        }
        return 0;
    }
}

int app_list_load(int list_id, const char *path_data, unsigned __int64 path_length) {
    unsigned char document[JADREN_APP_LIST_DOCUMENT_MAX];
    JadrenAppList parsed;
    unsigned __int64 document_length;
    unsigned int index;
    unsigned __int64 byte_index;
    if (!app_list_valid(list_id) || path_data == 0 ||
        file_size(path_data, path_length) > JADREN_APP_LIST_DOCUMENT_MAX) {
        return 0;
    }
    document_length = file_read(path_data, path_length, document, sizeof(document));
    app_list_clear_store(&parsed);
    if (!app_list_parse_document(document, document_length, &parsed)) {
        return 0;
    }
    jadren_app_lists[list_id].count = parsed.count;
    for (index = 0; index < JADREN_APP_LIST_MAX_ITEMS; index += 1) {
        jadren_app_lists[list_id].items[index].length = parsed.items[index].length;
        for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1) {
            jadren_app_lists[list_id].items[index].text[byte_index] =
                parsed.items[index].text[byte_index];
        }
    }
    return 1;
}

/* Import one caller-owned JSON list transactionally. The bounded parser fills
 * a temporary store first, so malformed or truncated input leaves the live
 * list unchanged. */
int app_list_import_json_exact(int list_id, const unsigned char *input_data,
                               unsigned __int64 input_capacity,
                               unsigned __int64 input_length) {
    JadrenAppList parsed;
    if (!app_list_valid(list_id) || input_data == 0 || input_length == 0 ||
        input_length > input_capacity || input_length > JADREN_APP_LIST_DOCUMENT_MAX) {
        return 0;
    }
    app_list_clear_store(&parsed);
    if (!app_list_parse_document(input_data, input_length, &parsed)) return 0;
    app_list_copy_store(&jadren_app_lists[list_id], &parsed);
    return 1;
}

/* Import one JSON list only when the caller's equality-only model revision is
 * still current. The underlying import remains transactional. */
int app_list_import_json_exact_if_revision(
    int list_id, const unsigned char *input_data,
    unsigned __int64 input_capacity, unsigned __int64 input_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_import_json_exact(
        list_id, input_data, input_capacity, input_length);
}

/* Import one bounded JSON list file transactionally. The file is read into a
 * fixed scratch buffer and published only after the complete document parses. */
int app_list_import_json_file(int list_id, const char *path_data,
                              unsigned __int64 path_length) {
    static unsigned char document[JADREN_APP_LIST_DOCUMENT_MAX];
    JadrenAppList parsed;
    unsigned __int64 document_length;
    unsigned __int64 file_length;
    if (!app_list_valid(list_id) || path_data == 0 || path_length == 0) return 0;
    file_length = file_size(path_data, path_length);
    if (file_length == 0 || file_length > sizeof(document)) return 0;
    document_length = file_read(path_data, path_length, document, sizeof(document));
    if (document_length != file_length) return 0;
    app_list_clear_store(&parsed);
    if (!app_list_parse_document(document, document_length, &parsed)) return 0;
    app_list_copy_store(&jadren_app_lists[list_id], &parsed);
    return 1;
}

/* Import one JSON list file only when the caller's equality-only model
 * revision is still current. The underlying file parser remains transactional. */
int app_list_import_json_file_if_revision(
    int list_id, const char *path_data, unsigned __int64 path_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_import_json_file(list_id, path_data, path_length);
}

#define JADREN_APP_TABLE_MAX_TABLES 4
#define JADREN_APP_TABLE_MAX_ROWS 64
#define JADREN_APP_TABLE_MAX_COLUMNS 8
#define JADREN_APP_TABLE_COLUMN_NAME_MAX 64
#define JADREN_APP_TABLE_MIGRATION_MAX_OPS 16
#define JADREN_APP_TABLE_CSV_INPUT_MAX 524288

enum {
    JADREN_APP_TABLE_TEXT = 0,
    JADREN_APP_TABLE_INT = 1,
    JADREN_APP_TABLE_UINT = 2,
    JADREN_APP_TABLE_BOOL = 3,
    JADREN_APP_TABLE_FLOAT = 4
};

typedef struct JadrenAppTable {
    unsigned __int64 row_count;
    int schema_version;
    unsigned char column_types[JADREN_APP_TABLE_MAX_COLUMNS];
    JadrenAppListItem column_names[JADREN_APP_TABLE_MAX_COLUMNS];
    JadrenAppListItem cells[JADREN_APP_TABLE_MAX_ROWS][JADREN_APP_TABLE_MAX_COLUMNS];
} JadrenAppTable;

typedef struct JadrenAppTableIndex {
    int active;
    int column_index;
    int kind;
    unsigned int row_count;
    unsigned int fingerprint;
    unsigned char rows[JADREN_APP_TABLE_MAX_ROWS];
} JadrenAppTableIndex;
typedef struct JadrenAppTablePairIndex {
    int active;
    int first_column_index;
    int second_column_index;
    unsigned int row_count;
    unsigned int fingerprint;
    unsigned char rows[JADREN_APP_TABLE_MAX_ROWS];
} JadrenAppTablePairIndex;
typedef struct JadrenAppTableMigrationOp {
    int kind;
    unsigned __int64 first_length;
    unsigned __int64 second_length;
    unsigned char first[JADREN_APP_LIST_TEXT_MAX];
    unsigned char second[JADREN_APP_LIST_TEXT_MAX];
    int column_kind;
} JadrenAppTableMigrationOp;

typedef struct JadrenAppTableMigration {
    int active;
    int table_id;
    int expected_version;
    int target_version;
    int operation_count;
    JadrenAppTableMigrationOp operations[JADREN_APP_TABLE_MIGRATION_MAX_OPS];
} JadrenAppTableMigration;

static int app_table_column_name_valid(const unsigned char *name_data,
                                      unsigned __int64 name_length);
static int app_table_column_name_matches(const JadrenAppListItem *item,
                                         const unsigned char *name_data,
                                         unsigned __int64 name_length);
int app_table_set_column_name(int table_id, int column_index,
                              const char *name_data, unsigned __int64 name_length);
int app_table_find_column(int table_id, const char *name_data,
                          unsigned __int64 name_length);
int app_table_set_column_type(int table_id, int column_index, int kind);

static JadrenAppTable jadren_app_tables[JADREN_APP_TABLE_MAX_TABLES];
static JadrenAppTableIndex jadren_app_table_indexes[JADREN_APP_TABLE_MAX_TABLES];
static JadrenAppTablePairIndex jadren_app_table_pair_indexes[JADREN_APP_TABLE_MAX_TABLES];
static JadrenAppStateEntry jadren_app_data_state_backup[JADREN_APP_STATE_MAX_ENTRIES];
static JadrenAppList jadren_app_data_lists_backup[JADREN_APP_LIST_MAX_LISTS];
static JadrenAppTable jadren_app_data_tables_backup[JADREN_APP_TABLE_MAX_TABLES];
static JadrenAppTable jadren_app_table_transaction_backup;
static JadrenAppTable jadren_app_table_transaction_backups[JADREN_APP_TABLE_MAX_TABLES];
static int jadren_app_table_transaction_active;
static int jadren_app_table_transaction_id;
static int jadren_app_table_transaction_all;
static JadrenAppTableMigration jadren_app_table_migration;

static int app_table_valid(int table_id) {
    return table_id >= 0 && table_id < JADREN_APP_TABLE_MAX_TABLES;
}

static int app_table_parse_integer_text(const JadrenAppListItem *cell,
                                        int allow_negative,
                                        long long *signed_result,
                                        unsigned __int64 *unsigned_result) {
    unsigned __int64 index = 0;
    unsigned __int64 value = 0;
    unsigned __int64 limit = (unsigned __int64)-1;
    int negative = 0;
    if (cell == 0 || cell->length == 0) return 0;
    if (allow_negative && cell->text[0] == '-') {
        negative = 1;
        index = 1;
        limit = ((unsigned __int64)1 << 63);
    } else if (cell->text[0] == '+') {
        index = 1;
    }
    if (index >= cell->length) return 0;
    for (; index < cell->length; index += 1) {
        unsigned char digit = cell->text[index];
        if (digit < '0' || digit > '9') return 0;
        if (value > (limit - (digit - '0')) / 10) return 0;
        value = value * 10 + (digit - '0');
    }
    if (signed_result != 0) {
        if (negative && value == ((unsigned __int64)1 << 63)) {
            *signed_result = (-9223372036854775807LL - 1LL);
        } else {
            *signed_result = negative ? -(long long)value : (long long)value;
        }
    }
    if (unsigned_result != 0) *unsigned_result = value;
    return 1;
}

static int app_table_integer_text_valid(const JadrenAppListItem *cell, int allow_negative) {
    long long signed_result;
    unsigned __int64 unsigned_result;
    if (cell == 0 || cell->length == 0) return 1;
    return app_table_parse_integer_text(cell, allow_negative, &signed_result,
                                        &unsigned_result);
}

static int app_table_value_valid(const JadrenAppListItem *cell, unsigned char kind) {
    double float_result;
    if (cell == 0 || kind > JADREN_APP_TABLE_FLOAT) return 0;
    if (kind == JADREN_APP_TABLE_TEXT || cell->length == 0) return 1;
    if (kind == JADREN_APP_TABLE_BOOL) {
        return (cell->length == 4 && cell->text[0] == 't' && cell->text[1] == 'r' &&
                cell->text[2] == 'u' && cell->text[3] == 'e') ||
               (cell->length == 5 && cell->text[0] == 'f' && cell->text[1] == 'a' &&
                cell->text[2] == 'l' && cell->text[3] == 's' && cell->text[4] == 'e');
    }
    if (kind == JADREN_APP_TABLE_FLOAT) {
        return json_read_float_value(cell->text, 0, cell->length, &float_result);
    }
    return app_table_integer_text_valid(cell, kind == JADREN_APP_TABLE_INT);
}

static int app_table_store_valid(const JadrenAppTable *table) {
    unsigned int row_index;
    unsigned int column_index;
    if (table == 0) return 0;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        unsigned int other;
        if (!app_table_column_name_valid(table->column_names[column_index].text,
                                         table->column_names[column_index].length)) return 0;
        if (table->column_names[column_index].length == 0) continue;
        for (other = column_index + 1; other < JADREN_APP_TABLE_MAX_COLUMNS; other += 1) {
            if (app_table_column_name_matches(&table->column_names[column_index],
                                              table->column_names[other].text,
                                              table->column_names[other].length)) return 0;
        }
    }
    for (row_index = 0; row_index < table->row_count; row_index += 1) {
        for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
            if (!app_table_value_valid(&table->cells[row_index][column_index],
                                       table->column_types[column_index])) return 0;
        }
    }
    return 1;
}
static void app_table_copy_schema(JadrenAppTable *destination,
                                  const JadrenAppTable *source) {
    unsigned int column_index;
    unsigned __int64 byte_index;
    if (destination == 0 || source == 0) return;
    destination->schema_version = source->schema_version;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        destination->column_types[column_index] = source->column_types[column_index];
        destination->column_names[column_index].length = source->column_names[column_index].length;
        for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1) {
            destination->column_names[column_index].text[byte_index] =
                source->column_names[column_index].text[byte_index];
        }
    }
}

static void app_table_clear_row(JadrenAppTable *table, unsigned int row_index) {
    unsigned int column_index;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        app_list_clear_item(&table->cells[row_index][column_index]);
    }
}

static void app_table_clear_store(JadrenAppTable *table) {
    unsigned int row_index;
    if (table == 0) {
        return;
    }
    table->row_count = 0;
    for (row_index = 0; row_index < JADREN_APP_TABLE_MAX_ROWS; row_index += 1) {
        app_table_clear_row(table, row_index);
    }
}

void app_table_clear(int table_id) {
    if (!app_table_valid(table_id)) {
        return;
    }
    app_table_clear_store(&jadren_app_tables[table_id]);
}

int app_table_row_count(int table_id) {
    if (!app_table_valid(table_id)) {
        return 0;
    }
    return (int)jadren_app_tables[table_id].row_count;
}

int app_table_schema_version(int table_id) {
    if (!app_table_valid(table_id)) return -1;
    return jadren_app_tables[table_id].schema_version;
}

int app_table_set_schema_version(int table_id, int version) {
    if (!app_table_valid(table_id) || version < 0) return 0;
    jadren_app_tables[table_id].schema_version = version;
    return 1;
}

static int app_table_column_name_valid(const unsigned char *name_data,
                                      unsigned __int64 name_length) {
    unsigned __int64 index;
    if (name_length == 0) return name_data == 0 || name_length == 0;
    if (name_data == 0 || name_length > JADREN_APP_TABLE_COLUMN_NAME_MAX) return 0;
    if (!((name_data[0] >= 'A' && name_data[0] <= 'Z') ||
          (name_data[0] >= 'a' && name_data[0] <= 'z') || name_data[0] == '_')) return 0;
    for (index = 1; index < name_length; index += 1) {
        unsigned char value = name_data[index];
        if (!((value >= 'A' && value <= 'Z') || (value >= 'a' && value <= 'z') ||
              (value >= '0' && value <= '9') || value == '_')) return 0;
    }
    return 1;
}

static int app_table_column_name_matches(const JadrenAppListItem *item,
                                         const unsigned char *name_data,
                                         unsigned __int64 name_length) {
    unsigned __int64 index;
    if (item == 0 || item->length != name_length) return 0;
    for (index = 0; index < name_length; index += 1) {
        if (item->text[index] != name_data[index]) return 0;
    }
    return 1;
}

static int app_table_column_name_unique(int table_id, int column_index,
                                       const unsigned char *name_data,
                                       unsigned __int64 name_length) {
    int other;
    if (name_length == 0) return 1;
    for (other = 0; other < JADREN_APP_TABLE_MAX_COLUMNS; other += 1) {
        if (other != column_index && app_table_column_name_matches(
                                         &jadren_app_tables[table_id].column_names[other],
                                         name_data, name_length)) return 0;
    }
    return 1;
}

int app_table_set_column_name(int table_id, int column_index,
                              const char *name_data, unsigned __int64 name_length) {
    unsigned __int64 byte_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        (name_data == 0 && name_length > 0) ||
        !app_table_column_name_valid((const unsigned char *)name_data, name_length) ||
        !app_table_column_name_unique(table_id, column_index,
                                      (const unsigned char *)name_data, name_length)) return 0;
    jadren_app_tables[table_id].column_names[column_index].length = name_length;
    for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1) {
        jadren_app_tables[table_id].column_names[column_index].text[byte_index] =
            byte_index < name_length ? (unsigned char)name_data[byte_index] : 0;
    }
    return 1;
}

unsigned __int64 app_table_read_column_name(int table_id, int column_index,
                                            unsigned char *output_data,
                                            unsigned __int64 output_length) {
    unsigned __int64 byte_index;
    JadrenAppListItem *name;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS || output_data == 0) return 0;
    name = &jadren_app_tables[table_id].column_names[column_index];
    if (name->length == 0 || output_length < name->length) return 0;
    for (byte_index = 0; byte_index < name->length; byte_index += 1)
        output_data[byte_index] = name->text[byte_index];
    return name->length;
}

int app_table_read_column_name_exact(int table_id, int column_index,
                                     unsigned char *output_data,
                                     unsigned __int64 output_length,
                                     unsigned __int64 *output_text_length,
                                     unsigned __int64 output_text_length_capacity) {
    JadrenAppListItem *name;
    unsigned __int64 byte_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        output_text_length == 0 || output_text_length_capacity == 0) return 0;
    name = &jadren_app_tables[table_id].column_names[column_index];
    if ((output_data == 0 && name->length > 0) || output_length < name->length) return 0;
    for (byte_index = 0; byte_index < name->length; byte_index += 1)
        output_data[byte_index] = name->text[byte_index];
    output_text_length[0] = name->length;
    return 1;
}

int app_table_find_column(int table_id, const char *name_data,
                          unsigned __int64 name_length) {
    int column_index;
    if (!app_table_valid(table_id) || name_length == 0 ||
        !app_table_column_name_valid((const unsigned char *)name_data, name_length)) return -1;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        if (app_table_column_name_matches(
                &jadren_app_tables[table_id].column_names[column_index],
                (const unsigned char *)name_data, name_length)) return column_index;
    }
    return -1;
}

int app_table_set_column_type(int table_id, int column_index, int kind) {
    unsigned char previous;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS || kind < JADREN_APP_TABLE_TEXT ||
        kind > JADREN_APP_TABLE_FLOAT) return 0;
    previous = jadren_app_tables[table_id].column_types[column_index];
    jadren_app_tables[table_id].column_types[column_index] = (unsigned char)kind;
    if (!app_table_store_valid(&jadren_app_tables[table_id])) {
        jadren_app_tables[table_id].column_types[column_index] = previous;
        return 0;
    }
    return 1;
}

int app_table_column_type(int table_id, int column_index) {
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS) return -1;
    return (int)jadren_app_tables[table_id].column_types[column_index];
}

int app_table_validate(int table_id) {
    if (!app_table_valid(table_id)) return 0;
    return app_table_store_valid(&jadren_app_tables[table_id]);
}

int app_table_append_row(int table_id) {
    unsigned int row_index;
    if (!app_table_valid(table_id) ||
        jadren_app_tables[table_id].row_count >= JADREN_APP_TABLE_MAX_ROWS) {
        return 0;
    }
    row_index = (unsigned int)jadren_app_tables[table_id].row_count;
    app_table_clear_row(&jadren_app_tables[table_id], row_index);
    jadren_app_tables[table_id].row_count += 1;
    return 1;
}

/* Append one table row only when the caller's equality-only model revision is
 * still current. This is process-local coordination, not a cross-thread
 * atomic or persistence primitive. */
int app_table_append_row_if_revision(int table_id, unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_append_row(table_id);
}

int app_table_remove_row(int table_id, int row_index) {
    unsigned __int64 index;
    unsigned int column_index;
    if (!app_table_valid(table_id) || row_index < 0 ||
        (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count) {
        return 0;
    }
    for (index = (unsigned __int64)row_index;
         index + 1 < jadren_app_tables[table_id].row_count; index += 1) {
        for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
            unsigned __int64 byte_index;
            JadrenAppListItem *destination = &jadren_app_tables[table_id].cells[index][column_index];
            JadrenAppListItem *source = &jadren_app_tables[table_id].cells[index + 1][column_index];
            destination->length = source->length;
            for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1) {
                destination->text[byte_index] = source->text[byte_index];
            }
        }
    }
    jadren_app_tables[table_id].row_count -= 1;
    app_table_clear_row(&jadren_app_tables[table_id],
                        (unsigned int)jadren_app_tables[table_id].row_count);
    return 1;
}

/* Remove one table row only when the caller's equality-only model revision is
 * still current. This is process-local coordination, not a cross-thread
 * atomic or persistence primitive. */
int app_table_remove_row_if_revision(int table_id, int row_index,
                                     unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_remove_row(table_id, row_index);
}

int app_table_set_cell(int table_id, int row_index, int column_index,
                       const char *value_data, unsigned __int64 value_length) {
    JadrenAppListItem *cell;
    JadrenAppListItem candidate;
    unsigned __int64 byte_index;
    if (!app_table_valid(table_id) || row_index < 0 || column_index < 0 ||
        (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        (value_data == 0 && value_length > 0) ||
        value_length > JADREN_APP_LIST_TEXT_MAX) {
        return 0;
    }
    candidate.length = value_length;
    for (byte_index = 0; byte_index < value_length; byte_index += 1) {
        candidate.text[byte_index] = (unsigned char)value_data[byte_index];
    }
    if (!app_table_value_valid(&candidate, jadren_app_tables[table_id].column_types[column_index])) return 0;
    cell = &jadren_app_tables[table_id].cells[row_index][column_index];
    cell->length = value_length;
    for (byte_index = 0; byte_index < value_length; byte_index += 1)
        cell->text[byte_index] = candidate.text[byte_index];
    return 1;
}

/* Update one table cell only when the caller's equality-only model revision
 * is still current. */
int app_table_set_cell_if_revision(int table_id, int row_index, int column_index,
                                   const char *value_data,
                                   unsigned __int64 value_length,
                                   unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_set_cell(table_id, row_index, column_index, value_data, value_length);
}

int app_table_set_cell_bytes(int table_id, int row_index, int column_index,
                             const unsigned char *value_data,
                             unsigned __int64 value_length) {
    return app_table_set_cell(table_id, row_index, column_index,
                              (const char *)value_data, value_length);
}

int app_table_set_cell_bytes_ex(int table_id, int row_index, int column_index,
                                const unsigned char *value_data,
                                unsigned __int64 value_capacity,
                                unsigned __int64 value_length) {
    if (value_length > value_capacity) return 0;
    return app_table_set_cell_bytes(table_id, row_index, column_index,
                                    value_data, value_length);
}

/* Caller-owned byte-prefix replacement guarded by the equality-only model
 * revision. Typed validation remains in the underlying table setter. */
int app_table_set_cell_bytes_if_revision(int table_id, int row_index, int column_index,
                                         const unsigned char *value_data,
                                         unsigned __int64 value_capacity,
                                         unsigned __int64 value_length,
                                         unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision || value_length > value_capacity) return 0;
    return app_table_set_cell_bytes_ex(table_id, row_index, column_index,
                                       value_data, value_capacity, value_length);
}

int app_table_set_int(int table_id, int row_index, int column_index, long long value) {
    unsigned char value_data[24];
    unsigned __int64 value_length = format_int(value, value_data, sizeof(value_data));
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_INT) return 0;
    if (value_length == 0) return 0;
    return app_table_set_cell(table_id, row_index, column_index,
                              (const char *)value_data, value_length);
}

int app_table_set_uint(int table_id, int row_index, int column_index,
                       unsigned __int64 value) {
    unsigned char value_data[24];
    unsigned __int64 value_length = format_uint(value, value_data, sizeof(value_data));
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_UINT) return 0;
    if (value_length == 0) return 0;
    return app_table_set_cell(table_id, row_index, column_index,
                              (const char *)value_data, value_length);
}

int app_table_set_float(int table_id, int row_index, int column_index, double value) {
    unsigned char value_data[32];
    unsigned __int64 value_length = format_float(value, value_data, sizeof(value_data));
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_FLOAT) return 0;
    if (value_length == 0) return 0;
    return app_table_set_cell(table_id, row_index, column_index,
                              (const char *)value_data, value_length);
}

int app_table_set_bool(int table_id, int row_index, int column_index, unsigned char value) {
    unsigned char value_data[5];
    unsigned __int64 value_length = format_bool(value, value_data, sizeof(value_data));
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_BOOL) return 0;
    if (value_length == 0) return 0;
    return app_table_set_cell(table_id, row_index, column_index,
                              (const char *)value_data, value_length);
}

unsigned __int64 app_table_read_cell(int table_id, int row_index, int column_index,
                                     unsigned char *output_data,
                                     unsigned __int64 output_length) {
    JadrenAppListItem *cell;
    unsigned __int64 byte_index;
    if (!app_table_valid(table_id) || row_index < 0 || column_index < 0 ||
        (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS || output_data == 0) {
        return 0;
    }
    cell = &jadren_app_tables[table_id].cells[row_index][column_index];
    if (cell->length == 0 || output_length < cell->length) {
        return 0;
    }
    for (byte_index = 0; byte_index < cell->length; byte_index += 1) {
        output_data[byte_index] = cell->text[byte_index];
    }
    return cell->length;
}

int app_table_read_cell_exact(int table_id, int row_index, int column_index,
                              unsigned char *output_data,
                              unsigned __int64 output_length,
                              unsigned __int64 *output_text_length,
                              unsigned __int64 output_text_length_capacity) {
    JadrenAppListItem *cell;
    unsigned __int64 byte_index;
    if (!app_table_valid(table_id) || row_index < 0 || column_index < 0 ||
        (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        output_text_length == 0 || output_text_length_capacity == 0) return 0;
    cell = &jadren_app_tables[table_id].cells[row_index][column_index];
    if ((output_data == 0 && cell->length > 0) || output_length < cell->length) return 0;
    for (byte_index = 0; byte_index < cell->length; byte_index += 1)
        output_data[byte_index] = cell->text[byte_index];
    output_text_length[0] = cell->length;
    return 1;
}

long long app_table_read_int(int table_id, int row_index, int column_index) {
    long long value = 0;
    if (!app_table_valid(table_id) || row_index < 0 || column_index < 0 ||
        (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_INT) {
        return 0;
    }
    if (!app_table_parse_integer_text(
            &jadren_app_tables[table_id].cells[row_index][column_index], 1, &value, 0)) {
        return 0;
    }
    return value;
}

unsigned __int64 app_table_read_uint(int table_id, int row_index, int column_index) {
    unsigned __int64 value = 0;
    if (!app_table_valid(table_id) || row_index < 0 || column_index < 0 ||
        (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_UINT) {
        return 0;
    }
    if (!app_table_parse_integer_text(
            &jadren_app_tables[table_id].cells[row_index][column_index], 0, 0, &value)) {
        return 0;
    }
    return value;
}

double app_table_read_float(int table_id, int row_index, int column_index) {
    double value = 0.0;
    JadrenAppListItem *cell;
    if (!app_table_valid(table_id) || row_index < 0 || column_index < 0 ||
        (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_FLOAT) return 0.0;
    cell = &jadren_app_tables[table_id].cells[row_index][column_index];
    if (!json_read_float_value(cell->text, 0, cell->length, &value)) return 0.0;
    return value;
}

unsigned char app_table_read_bool(int table_id, int row_index, int column_index) {
    JadrenAppListItem *cell;
    if (!app_table_valid(table_id) || row_index < 0 || column_index < 0 ||
        (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_BOOL) {
        return 0;
    }
    cell = &jadren_app_tables[table_id].cells[row_index][column_index];
    return (cell->length == 4 && cell->text[0] == 't' && cell->text[1] == 'r' &&
            cell->text[2] == 'u' && cell->text[3] == 'e') ? 1 : 0;
}

int app_table_read_int_exact(int table_id, int row_index, int column_index,
                             long long *output_data, unsigned __int64 output_length) {
    long long value = 0;
    if (output_data == 0 || output_length == 0 || !app_table_valid(table_id) || row_index < 0 ||
        column_index < 0 || (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_INT ||
        !app_table_parse_integer_text(&jadren_app_tables[table_id].cells[row_index][column_index], 1, &value, 0)) return 0;
    output_data[0] = value;
    return 1;
}

int app_table_read_uint_exact(int table_id, int row_index, int column_index,
                              unsigned __int64 *output_data, unsigned __int64 output_length) {
    unsigned __int64 value = 0;
    if (output_data == 0 || output_length == 0 || !app_table_valid(table_id) || row_index < 0 ||
        column_index < 0 || (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_UINT ||
        !app_table_parse_integer_text(&jadren_app_tables[table_id].cells[row_index][column_index], 0, 0, &value)) return 0;
    output_data[0] = value;
    return 1;
}

int app_table_read_float_exact(int table_id, int row_index, int column_index,
                               double *output_data, unsigned __int64 output_length) {
    double value = 0.0;
    JadrenAppListItem *cell;
    if (output_data == 0 || output_length == 0 || !app_table_valid(table_id) || row_index < 0 ||
        column_index < 0 || (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_FLOAT) return 0;
    cell = &jadren_app_tables[table_id].cells[row_index][column_index];
    if (!json_read_float_value(cell->text, 0, cell->length, &value)) return 0;
    output_data[0] = value;
    return 1;
}

int app_table_read_bool_exact(int table_id, int row_index, int column_index,
                              unsigned char *output_data, unsigned __int64 output_length) {
    JadrenAppListItem *cell;
    unsigned char value;
    if (output_data == 0 || output_length == 0 || !app_table_valid(table_id) || row_index < 0 ||
        column_index < 0 || (unsigned __int64)row_index >= jadren_app_tables[table_id].row_count ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_BOOL) return 0;
    cell = &jadren_app_tables[table_id].cells[row_index][column_index];
    if (cell->length == 4 && cell->text[0] == 't' && cell->text[1] == 'r' && cell->text[2] == 'u' && cell->text[3] == 'e') value = 1;
    else if (cell->length == 5 && cell->text[0] == 'f' && cell->text[1] == 'a' && cell->text[2] == 'l' && cell->text[3] == 's' && cell->text[4] == 'e') value = 0;
    else return 0;
    output_data[0] = value;
    return 1;
}

int app_table_set_named_cell(int table_id, int row_index,
                             const char *column_name_data,
                             unsigned __int64 column_name_length,
                             const char *value_data,
                             unsigned __int64 value_length) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_set_cell(table_id, row_index, column_index, value_data, value_length);
}

unsigned __int64 app_table_read_named_cell(int table_id, int row_index,
                                           const char *column_name_data,
                                           unsigned __int64 column_name_length,
                                           unsigned char *output_data,
                                           unsigned __int64 output_length) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_read_cell(table_id, row_index, column_index, output_data, output_length);
}

int app_table_read_named_cell_exact(int table_id, int row_index,
                                    const char *column_name_data,
                                    unsigned __int64 column_name_length,
                                    unsigned char *output_data,
                                    unsigned __int64 output_length,
                                    unsigned __int64 *output_text_length,
                                    unsigned __int64 output_text_length_capacity) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_read_cell_exact(table_id, row_index, column_index,
                                     output_data, output_length,
                                     output_text_length, output_text_length_capacity);
}

int app_table_set_named_int(int table_id, int row_index,
                            const char *column_name_data,
                            unsigned __int64 column_name_length,
                            long long value) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_set_int(table_id, row_index, column_index, value);
}

int app_table_set_named_uint(int table_id, int row_index,
                             const char *column_name_data,
                             unsigned __int64 column_name_length,
                             unsigned __int64 value) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_set_uint(table_id, row_index, column_index, value);
}

int app_table_set_named_float(int table_id, int row_index,
                              const char *column_name_data,
                              unsigned __int64 column_name_length,
                              double value) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_set_float(table_id, row_index, column_index, value);
}

int app_table_set_named_bool(int table_id, int row_index,
                             const char *column_name_data,
                             unsigned __int64 column_name_length,
                             unsigned char value) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_set_bool(table_id, row_index, column_index, value);
}

long long app_table_read_named_int(int table_id, int row_index,
                                   const char *column_name_data,
                                   unsigned __int64 column_name_length) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_read_int(table_id, row_index, column_index);
}

unsigned __int64 app_table_read_named_uint(int table_id, int row_index,
                                           const char *column_name_data,
                                           unsigned __int64 column_name_length) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_read_uint(table_id, row_index, column_index);
}

double app_table_read_named_float(int table_id, int row_index,
                                  const char *column_name_data,
                                  unsigned __int64 column_name_length) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0.0;
    return app_table_read_float(table_id, row_index, column_index);
}

unsigned char app_table_read_named_bool(int table_id, int row_index,
                                        const char *column_name_data,
                                        unsigned __int64 column_name_length) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_read_bool(table_id, row_index, column_index);
}

int app_table_read_named_int_exact(int table_id, int row_index, const char *column_name_data,
                                   unsigned __int64 column_name_length, long long *output_data,
                                   unsigned __int64 output_length) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_read_int_exact(table_id, row_index, column_index, output_data, output_length);
}

int app_table_read_named_uint_exact(int table_id, int row_index, const char *column_name_data,
                                    unsigned __int64 column_name_length, unsigned __int64 *output_data,
                                    unsigned __int64 output_length) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_read_uint_exact(table_id, row_index, column_index, output_data, output_length);
}

int app_table_read_named_float_exact(int table_id, int row_index, const char *column_name_data,
                                     unsigned __int64 column_name_length, double *output_data,
                                     unsigned __int64 output_length) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_read_float_exact(table_id, row_index, column_index, output_data, output_length);
}

int app_table_read_named_bool_exact(int table_id, int row_index, const char *column_name_data,
                                    unsigned __int64 column_name_length, unsigned char *output_data,
                                    unsigned __int64 output_length) {
    int column_index = app_table_find_column(table_id, column_name_data, column_name_length);
    if (column_index < 0) return 0;
    return app_table_read_bool_exact(table_id, row_index, column_index, output_data, output_length);
}

static int app_table_compare_items(const JadrenAppListItem *left,
                                   const JadrenAppListItem *right) {
    unsigned __int64 index;
    unsigned __int64 common_length = left->length < right->length ? left->length : right->length;
    for (index = 0; index < common_length; index += 1) {
        if (left->text[index] < right->text[index]) return -1;
        if (left->text[index] > right->text[index]) return 1;
    }
    if (left->length < right->length) return -1;
    if (left->length > right->length) return 1;
    return 0;
}

static unsigned int app_table_index_fingerprint(const JadrenAppTable *table) {
    unsigned int hash = 2166136261U;
    unsigned int row_index;
    unsigned int column_index;
    if (table == 0) return 0;
    hash ^= (unsigned int)table->row_count;
    hash *= 16777619U;
    hash ^= (unsigned int)(table->row_count >> 32);
    hash *= 16777619U;
    for (row_index = 0; row_index < table->row_count; row_index += 1) {
        for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
            const JadrenAppListItem *cell = &table->cells[row_index][column_index];
            unsigned __int64 byte_index;
            hash ^= (unsigned int)cell->length;
            hash *= 16777619U;
            hash ^= (unsigned int)(cell->length >> 32);
            hash *= 16777619U;
            for (byte_index = 0; byte_index < cell->length; byte_index += 1) {
                hash ^= cell->text[byte_index];
                hash *= 16777619U;
            }
        }
    }
    return hash;
}

static int app_table_index_matches(int table_id, int column_index, int expected_kind) {
    JadrenAppTableIndex *index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS) return 0;
    index = &jadren_app_table_indexes[table_id];
    return index->active && index->column_index == column_index &&
           (expected_kind < 0 || index->kind == expected_kind) &&
           index->fingerprint == app_table_index_fingerprint(&jadren_app_tables[table_id]);
}

static int app_table_pair_index_matches(int table_id, int first_column_index,
                                        int second_column_index) {
    JadrenAppTablePairIndex *index;
    if (!app_table_valid(table_id) || first_column_index < 0 ||
        first_column_index >= JADREN_APP_TABLE_MAX_COLUMNS || second_column_index < 0 ||
        second_column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        first_column_index == second_column_index) return 0;
    index = &jadren_app_table_pair_indexes[table_id];
    return index->active && index->first_column_index == first_column_index &&
           index->second_column_index == second_column_index &&
           jadren_app_tables[table_id].column_types[first_column_index] == JADREN_APP_TABLE_TEXT &&
           jadren_app_tables[table_id].column_types[second_column_index] == JADREN_APP_TABLE_TEXT &&
           index->fingerprint == app_table_index_fingerprint(&jadren_app_tables[table_id]);
}

static int app_table_compare_pair_rows(int table_id, int first_column_index,
                                       int second_column_index, unsigned int left_row,
                                       unsigned int right_row) {
    int comparison = app_table_compare_items(
        &jadren_app_tables[table_id].cells[left_row][first_column_index],
        &jadren_app_tables[table_id].cells[right_row][first_column_index]);
    if (comparison != 0) return comparison;
    return app_table_compare_items(
        &jadren_app_tables[table_id].cells[left_row][second_column_index],
        &jadren_app_tables[table_id].cells[right_row][second_column_index]);
}

int app_table_index_build_pair(int table_id, int first_column_index,
                               int second_column_index) {
    JadrenAppTablePairIndex *index;
    unsigned int row_index;
    if (!app_table_valid(table_id) || first_column_index < 0 ||
        first_column_index >= JADREN_APP_TABLE_MAX_COLUMNS || second_column_index < 0 ||
        second_column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        first_column_index == second_column_index ||
        jadren_app_tables[table_id].column_types[first_column_index] != JADREN_APP_TABLE_TEXT ||
        jadren_app_tables[table_id].column_types[second_column_index] != JADREN_APP_TABLE_TEXT ||
        !app_table_store_valid(&jadren_app_tables[table_id])) return 0;
    index = &jadren_app_table_pair_indexes[table_id];
    index->active = 0;
    index->first_column_index = first_column_index;
    index->second_column_index = second_column_index;
    index->row_count = (unsigned int)jadren_app_tables[table_id].row_count;
    index->fingerprint = app_table_index_fingerprint(&jadren_app_tables[table_id]);
    for (row_index = 0; row_index < index->row_count; row_index += 1)
        index->rows[row_index] = (unsigned char)row_index;
    for (row_index = 1; row_index < index->row_count; row_index += 1) {
        unsigned int current = index->rows[row_index];
        unsigned int cursor = row_index;
        while (cursor > 0) {
            unsigned int previous = index->rows[cursor - 1];
            int comparison = app_table_compare_pair_rows(
                table_id, first_column_index, second_column_index, previous, current);
            if (comparison < 0 || (comparison == 0 && previous < current)) break;
            index->rows[cursor] = (unsigned char)previous;
            cursor -= 1;
        }
        index->rows[cursor] = (unsigned char)current;
    }
    index->active = 1;
    return 1;
}

int app_table_index_find_pair_text(int table_id, int first_column_index,
                                   int second_column_index, const char *first_data,
                                   unsigned __int64 first_length, const char *second_data,
                                   unsigned __int64 second_length) {
    JadrenAppTablePairIndex *index;
    JadrenAppListItem first_query;
    JadrenAppListItem second_query;
    unsigned int low;
    unsigned int high;
    if ((first_data == 0 && first_length > 0) || (second_data == 0 && second_length > 0) ||
        first_length > JADREN_APP_LIST_TEXT_MAX || second_length > JADREN_APP_LIST_TEXT_MAX ||
        !app_table_pair_index_matches(table_id, first_column_index, second_column_index)) return -1;
    first_query.length = first_length;
    second_query.length = second_length;
    for (unsigned __int64 byte_index = 0; byte_index < first_length; byte_index += 1)
        first_query.text[byte_index] = (unsigned char)first_data[byte_index];
    for (unsigned __int64 byte_index = 0; byte_index < second_length; byte_index += 1)
        second_query.text[byte_index] = (unsigned char)second_data[byte_index];
    index = &jadren_app_table_pair_indexes[table_id];
    low = 0;
    high = index->row_count;
    while (low < high) {
        unsigned int middle = low + (high - low) / 2;
        unsigned int row = index->rows[middle];
        int comparison = app_table_compare_items(
            &jadren_app_tables[table_id].cells[row][first_column_index], &first_query);
        if (comparison == 0)
            comparison = app_table_compare_items(
                &jadren_app_tables[table_id].cells[row][second_column_index], &second_query);
        if (comparison < 0) low = middle + 1;
        else high = middle;
    }
    if (low >= index->row_count) return -1;
    {
        unsigned int row = index->rows[low];
        if (app_table_compare_items(&jadren_app_tables[table_id].cells[row][first_column_index],
                                    &first_query) != 0 ||
            app_table_compare_items(&jadren_app_tables[table_id].cells[row][second_column_index],
                                    &second_query) != 0) return -1;
        return (int)row;
    }
}

int app_table_index_find_pair_text_if_revision(
    int table_id, int first_column_index, int second_column_index,
    const char *first_data, unsigned __int64 first_length,
    const char *second_data, unsigned __int64 second_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return -1;
    return app_table_index_find_pair_text(
        table_id, first_column_index, second_column_index,
        first_data, first_length, second_data, second_length);
}

static int app_table_compare_typed_cells(int table_id, int column_index,
                                         unsigned int left_row, unsigned int right_row,
                                         int kind) {
    const JadrenAppListItem *left = &jadren_app_tables[table_id].cells[left_row][column_index];
    const JadrenAppListItem *right = &jadren_app_tables[table_id].cells[right_row][column_index];
    if (left->length == 0 && right->length == 0) return 0;
    if (left->length == 0) return -1;
    if (right->length == 0) return 1;
    if (kind == JADREN_APP_TABLE_INT) {
        long long left_value = app_table_read_int(table_id, (int)left_row, column_index);
        long long right_value = app_table_read_int(table_id, (int)right_row, column_index);
        return left_value < right_value ? -1 : (left_value > right_value ? 1 : 0);
    }
    if (kind == JADREN_APP_TABLE_UINT) {
        unsigned __int64 left_value = app_table_read_uint(table_id, (int)left_row, column_index);
        unsigned __int64 right_value = app_table_read_uint(table_id, (int)right_row, column_index);
        return left_value < right_value ? -1 : (left_value > right_value ? 1 : 0);
    }
    if (kind == JADREN_APP_TABLE_FLOAT) {
        double left_value = app_table_read_float(table_id, (int)left_row, column_index);
        double right_value = app_table_read_float(table_id, (int)right_row, column_index);
        return left_value < right_value ? -1 : (left_value > right_value ? 1 : 0);
    }
    if (kind == JADREN_APP_TABLE_BOOL) {
        int left_value = app_table_read_bool(table_id, (int)left_row, column_index);
        int right_value = app_table_read_bool(table_id, (int)right_row, column_index);
        return left_value < right_value ? -1 : (left_value > right_value ? 1 : 0);
    }
    return app_table_compare_items(left, right);
}

static int app_table_index_build_kind(int table_id, int column_index, int kind) {
    JadrenAppTableIndex *index;
    unsigned int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        !app_table_store_valid(&jadren_app_tables[table_id]) ||
        jadren_app_tables[table_id].column_types[column_index] != kind) return 0;
    index = &jadren_app_table_indexes[table_id];
    index->active = 0;
    index->column_index = column_index;
    index->kind = kind;
    index->row_count = (unsigned int)jadren_app_tables[table_id].row_count;
    index->fingerprint = app_table_index_fingerprint(&jadren_app_tables[table_id]);
    for (row_index = 0; row_index < index->row_count; row_index += 1)
        index->rows[row_index] = (unsigned char)row_index;
    for (row_index = 1; row_index < index->row_count; row_index += 1) {
        unsigned int current = index->rows[row_index];
        unsigned int cursor = row_index;
        while (cursor > 0) {
            unsigned int previous = index->rows[cursor - 1];
            int comparison = app_table_compare_typed_cells(
                table_id, column_index, previous, current, kind);
            if (comparison < 0 || (comparison == 0 && previous < current)) break;
            index->rows[cursor] = (unsigned char)previous;
            cursor -= 1;
        }
        index->rows[cursor] = (unsigned char)current;
    }
    index->active = 1;
    return 1;
}

int app_table_index_build(int table_id, int column_index) {
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS) return 0;
    return app_table_index_build_kind(table_id, column_index,
                                      jadren_app_tables[table_id].column_types[column_index]);
}

int app_table_index_build_int(int table_id, int column_index) {
    return app_table_index_build_kind(table_id, column_index, JADREN_APP_TABLE_INT);
}

int app_table_index_build_uint(int table_id, int column_index) {
    return app_table_index_build_kind(table_id, column_index, JADREN_APP_TABLE_UINT);
}

int app_table_index_build_float(int table_id, int column_index) {
    return app_table_index_build_kind(table_id, column_index, JADREN_APP_TABLE_FLOAT);
}

int app_table_index_build_bool(int table_id, int column_index) {
    return app_table_index_build_kind(table_id, column_index, JADREN_APP_TABLE_BOOL);
}

int app_table_index_clear(int table_id) {
    JadrenAppTableIndex *index;
    JadrenAppTablePairIndex *pair_index;
    if (!app_table_valid(table_id)) return 0;
    index = &jadren_app_table_indexes[table_id];
    pair_index = &jadren_app_table_pair_indexes[table_id];
    index->active = 0;
    index->column_index = -1;
    index->kind = -1;
    index->row_count = 0;
    index->fingerprint = 0;
    pair_index->active = 0;
    pair_index->first_column_index = -1;
    pair_index->second_column_index = -1;
    pair_index->row_count = 0;
    pair_index->fingerprint = 0;
    return 1;
}

int app_table_index_is_valid(int table_id, int column_index) {
    return app_table_index_matches(table_id, column_index, -1);
}

static void app_table_swap_rows(JadrenAppTable *table,
                                unsigned int left_row,
                                unsigned int right_row) {
    unsigned int column_index;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        JadrenAppListItem temporary;
        unsigned __int64 byte_index;
        JadrenAppListItem *left = &table->cells[left_row][column_index];
        JadrenAppListItem *right = &table->cells[right_row][column_index];
        temporary.length = left->length;
        for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1) {
            temporary.text[byte_index] = left->text[byte_index];
            left->text[byte_index] = right->text[byte_index];
            right->text[byte_index] = temporary.text[byte_index];
        }
        left->length = right->length;
        right->length = temporary.length;
    }
}

static void app_table_copy_row(JadrenAppTable *destination,
                               unsigned int destination_row,
                               const JadrenAppTable *source,
                               unsigned int source_row) {
    unsigned int column_index;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        unsigned __int64 byte_index;
        JadrenAppListItem *target = &destination->cells[destination_row][column_index];
        const JadrenAppListItem *origin = &source->cells[source_row][column_index];
        target->length = origin->length;
        for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1) {
            target->text[byte_index] = origin->text[byte_index];
        }
    }
}

static void app_table_copy_store(JadrenAppTable *destination,
                                 const JadrenAppTable *source) {
    unsigned int row_index;
    if (destination == 0 || source == 0) return;
    app_table_copy_schema(destination, source);
    destination->row_count = source->row_count;
    for (row_index = 0; row_index < JADREN_APP_TABLE_MAX_ROWS; row_index += 1) {
        app_table_copy_row(destination, row_index, source, row_index);
    }
}

static unsigned __int64 app_data_hash_bytes(unsigned __int64 hash,
                                             const unsigned char *data,
                                             unsigned __int64 length) {
    unsigned __int64 index;
    if (data == 0) return hash;
    for (index = 0; index < length; index += 1) {
        hash ^= data[index];
        hash *= 1099511628211ULL;
    }
    return hash;
}

static unsigned __int64 app_data_model_revision(void) {
    unsigned __int64 hash = 1469598103934665603ULL;
    unsigned int state_index;
    unsigned int list_index;
    unsigned int table_index;
    for (state_index = 0; state_index < JADREN_APP_STATE_MAX_ENTRIES; state_index += 1) {
        const JadrenAppStateEntry *entry = &jadren_app_state[state_index];
        hash = app_data_hash_bytes(hash, (const unsigned char *)&state_index, sizeof(state_index));
        hash = app_data_hash_bytes(hash, &entry->used, sizeof(entry->used));
        hash = app_data_hash_bytes(hash, &entry->kind, sizeof(entry->kind));
        if (!entry->used) continue;
        hash = app_data_hash_bytes(hash, (const unsigned char *)&entry->key_length, sizeof(entry->key_length));
        hash = app_data_hash_bytes(hash, entry->key, entry->key_length);
        if (entry->kind == JADREN_APP_STATE_INT) {
            hash = app_data_hash_bytes(hash, (const unsigned char *)&entry->int_value, sizeof(entry->int_value));
        } else if (entry->kind == JADREN_APP_STATE_UINT) {
            hash = app_data_hash_bytes(hash, (const unsigned char *)&entry->uint_value, sizeof(entry->uint_value));
        } else if (entry->kind == JADREN_APP_STATE_BOOL) {
            hash = app_data_hash_bytes(hash, &entry->bool_value, sizeof(entry->bool_value));
        } else if (entry->kind == JADREN_APP_STATE_FLOAT) {
            hash = app_data_hash_bytes(hash, (const unsigned char *)&entry->float_value, sizeof(entry->float_value));
        } else if (entry->kind == JADREN_APP_STATE_TEXT) {
            hash = app_data_hash_bytes(hash, (const unsigned char *)&entry->text_length, sizeof(entry->text_length));
            hash = app_data_hash_bytes(hash, entry->text, entry->text_length);
        }
    }
    for (list_index = 0; list_index < JADREN_APP_LIST_MAX_LISTS; list_index += 1) {
        const JadrenAppList *list = &jadren_app_lists[list_index];
        unsigned int item_index;
        hash = app_data_hash_bytes(hash, (const unsigned char *)&list_index, sizeof(list_index));
        hash = app_data_hash_bytes(hash, (const unsigned char *)&list->count, sizeof(list->count));
        for (item_index = 0; item_index < list->count; item_index += 1) {
            const JadrenAppListItem *item = &list->items[item_index];
            hash = app_data_hash_bytes(hash, (const unsigned char *)&item_index, sizeof(item_index));
            hash = app_data_hash_bytes(hash, (const unsigned char *)&item->length, sizeof(item->length));
            hash = app_data_hash_bytes(hash, item->text, item->length);
        }
    }
    for (table_index = 0; table_index < JADREN_APP_TABLE_MAX_TABLES; table_index += 1) {
        const JadrenAppTable *table = &jadren_app_tables[table_index];
        unsigned int column_index;
        hash = app_data_hash_bytes(hash, (const unsigned char *)&table_index, sizeof(table_index));
        hash = app_data_hash_bytes(hash, (const unsigned char *)&table->schema_version, sizeof(table->schema_version));
        hash = app_data_hash_bytes(hash, (const unsigned char *)&table->row_count, sizeof(table->row_count));
        for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
            const JadrenAppListItem *name = &table->column_names[column_index];
            hash = app_data_hash_bytes(hash, (const unsigned char *)&column_index, sizeof(column_index));
            hash = app_data_hash_bytes(hash, &table->column_types[column_index], sizeof(table->column_types[column_index]));
            hash = app_data_hash_bytes(hash, (const unsigned char *)&name->length, sizeof(name->length));
            hash = app_data_hash_bytes(hash, name->text, name->length);
        }
        {
            unsigned int row_index;
            for (row_index = 0; row_index < table->row_count; row_index += 1) {
                for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
                    const JadrenAppListItem *cell = &table->cells[row_index][column_index];
                    hash = app_data_hash_bytes(hash, (const unsigned char *)&row_index, sizeof(row_index));
                    hash = app_data_hash_bytes(hash, (const unsigned char *)&cell->length, sizeof(cell->length));
                    hash = app_data_hash_bytes(hash, cell->text, cell->length);
                }
            }
        }
    }
    return hash;
}

unsigned __int64 app_data_revision(void) {
    return app_data_model_revision();
}

static int app_data_validate_model(void) {
    unsigned int state_index;
    unsigned int list_index;
    unsigned int table_index;
    for (state_index = 0; state_index < JADREN_APP_STATE_MAX_ENTRIES; state_index += 1) {
        const JadrenAppStateEntry *entry = &jadren_app_state[state_index];
        if (!entry->used) continue;
        if (!app_state_key_is_safe(entry->key, entry->key_length)) return 0;
        if (entry->kind == JADREN_APP_STATE_TEXT &&
            entry->text_length > JADREN_APP_STATE_TEXT_MAX) return 0;
        if (entry->kind != JADREN_APP_STATE_INT &&
            entry->kind != JADREN_APP_STATE_UINT &&
            entry->kind != JADREN_APP_STATE_BOOL &&
            entry->kind != JADREN_APP_STATE_TEXT &&
            entry->kind != JADREN_APP_STATE_FLOAT) return 0;
    }
    for (list_index = 0; list_index < JADREN_APP_LIST_MAX_LISTS; list_index += 1) {
        const JadrenAppList *list = &jadren_app_lists[list_index];
        unsigned int item_index;
        if (list->count > JADREN_APP_LIST_MAX_ITEMS) return 0;
        for (item_index = 0; item_index < list->count; item_index += 1) {
            if (list->items[item_index].length > JADREN_APP_LIST_TEXT_MAX) return 0;
        }
    }
    for (table_index = 0; table_index < JADREN_APP_TABLE_MAX_TABLES; table_index += 1) {
        if (!app_table_validate((int)table_index)) return 0;
    }
    return 1;
}

int app_data_validate(void) {
    return app_data_validate_model();
}

int file_delete(const char *path_data, unsigned __int64 path_length);
int file_flush(const char *path_data, unsigned __int64 path_length);
int directory_flush(const char *path_data, unsigned __int64 path_length);
unsigned __int64 file_lock(const char *path_data, unsigned __int64 path_length);
int file_unlock(unsigned __int64 token);
int file_replace_atomic(const char *source_data, unsigned __int64 source_length,
                        const char *target_data, unsigned __int64 target_length);

int app_data_tx_begin(void) {
    unsigned int index;
    if (jadren_app_data_transaction_active || jadren_app_state_transaction_active ||
        jadren_app_table_transaction_active) return 0;
    for (index = 0; index < JADREN_APP_STATE_MAX_ENTRIES; index += 1)
        app_state_copy_entry(&jadren_app_data_state_backup[index], &jadren_app_state[index]);
    for (index = 0; index < JADREN_APP_LIST_MAX_LISTS; index += 1)
        app_list_copy_store(&jadren_app_data_lists_backup[index], &jadren_app_lists[index]);
    for (index = 0; index < JADREN_APP_TABLE_MAX_TABLES; index += 1)
        app_table_copy_store(&jadren_app_data_tables_backup[index], &jadren_app_tables[index]);
    jadren_app_data_transaction_active = 1;
    return 1;
}

int app_data_tx_begin_if_revision(unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_data_tx_begin();
}

int app_data_tx_commit(void) {
    if (!jadren_app_data_transaction_active) return 0;
    app_state_clear_entries(jadren_app_data_state_backup);
    jadren_app_data_transaction_active = 0;
    return 1;
}

int app_data_tx_rollback(void) {
    unsigned int index;
    if (!jadren_app_data_transaction_active) return 0;
    for (index = 0; index < JADREN_APP_STATE_MAX_ENTRIES; index += 1)
        app_state_copy_entry(&jadren_app_state[index], &jadren_app_data_state_backup[index]);
    for (index = 0; index < JADREN_APP_LIST_MAX_LISTS; index += 1)
        app_list_copy_store(&jadren_app_lists[index], &jadren_app_data_lists_backup[index]);
    for (index = 0; index < JADREN_APP_TABLE_MAX_TABLES; index += 1)
        app_table_copy_store(&jadren_app_tables[index], &jadren_app_data_tables_backup[index]);
    app_state_clear_entries(jadren_app_data_state_backup);
    jadren_app_data_transaction_active = 0;
    app_state_bump_revision();
    app_data_refresh_bound_ui();
    return 1;
}

int app_table_tx_begin(int table_id) {
    if (!app_table_valid(table_id) || jadren_app_table_transaction_active ||
        jadren_app_data_transaction_active) return 0;
    app_table_copy_store(&jadren_app_table_transaction_backup, &jadren_app_tables[table_id]);
    jadren_app_table_transaction_id = table_id;
    jadren_app_table_transaction_all = 0;
    jadren_app_table_transaction_active = 1;
    return 1;
}

int app_table_tx_begin_all(void) {
    int table_id;
    if (jadren_app_table_transaction_active || jadren_app_data_transaction_active) return 0;
    for (table_id = 0; table_id < JADREN_APP_TABLE_MAX_TABLES; table_id += 1)
        app_table_copy_store(&jadren_app_table_transaction_backups[table_id],
                             &jadren_app_tables[table_id]);
    jadren_app_table_transaction_id = -1;
    jadren_app_table_transaction_all = 1;
    jadren_app_table_transaction_active = 1;
    return 1;
}

int app_table_tx_commit(void) {
    if (!jadren_app_table_transaction_active || jadren_app_table_transaction_all) return 0;
    jadren_app_table_transaction_active = 0;
    jadren_app_table_transaction_id = -1;
    jadren_app_table_transaction_all = 0;
    return 1;
}

int app_table_tx_commit_all(void) {
    if (!jadren_app_table_transaction_active || !jadren_app_table_transaction_all) return 0;
    jadren_app_table_transaction_active = 0;
    jadren_app_table_transaction_id = -1;
    jadren_app_table_transaction_all = 0;
    return 1;
}

int app_table_tx_rollback(void) {
    if (!jadren_app_table_transaction_active ||
        jadren_app_table_transaction_all ||
        !app_table_valid(jadren_app_table_transaction_id)) return 0;
    app_table_copy_store(&jadren_app_tables[jadren_app_table_transaction_id],
                         &jadren_app_table_transaction_backup);
    jadren_app_table_transaction_active = 0;
    jadren_app_table_transaction_id = -1;
    jadren_app_table_transaction_all = 0;
    app_data_refresh_bound_ui();
    return 1;
}

int app_table_tx_rollback_all(void) {
    int table_id;
    if (!jadren_app_table_transaction_active || !jadren_app_table_transaction_all) return 0;
    for (table_id = 0; table_id < JADREN_APP_TABLE_MAX_TABLES; table_id += 1)
        app_table_copy_store(&jadren_app_tables[table_id],
                             &jadren_app_table_transaction_backups[table_id]);
    jadren_app_table_transaction_active = 0;
    jadren_app_table_transaction_id = -1;
    jadren_app_table_transaction_all = 0;
    app_data_refresh_bound_ui();
    return 1;
}

static void app_table_migration_reset(void) {
    jadren_app_table_migration.active = 0;
    jadren_app_table_migration.table_id = -1;
    jadren_app_table_migration.expected_version = -1;
    jadren_app_table_migration.target_version = -1;
    jadren_app_table_migration.operation_count = 0;
}

int app_table_migration_begin(int table_id, int expected_version, int target_version) {
    if (!app_table_valid(table_id) || expected_version < 0 || target_version < 0 ||
        expected_version == target_version || jadren_app_table_migration.active ||
        jadren_app_data_transaction_active || jadren_app_table_transaction_active ||
        jadren_app_tables[table_id].schema_version != expected_version ||
        !app_table_tx_begin(table_id)) return 0;
    jadren_app_table_migration.active = 1;
    jadren_app_table_migration.table_id = table_id;
    jadren_app_table_migration.expected_version = expected_version;
    jadren_app_table_migration.target_version = target_version;
    jadren_app_table_migration.operation_count = 0;
    return 1;
}

int app_table_migration_rename_column(const char *from_data,
                                      unsigned __int64 from_length,
                                      const char *to_data,
                                      unsigned __int64 to_length) {
    JadrenAppTableMigrationOp *operation;
    unsigned __int64 byte_index;
    if (!jadren_app_table_migration.active || from_data == 0 || to_data == 0 ||
        from_length == 0 || to_length == 0 || from_length > JADREN_APP_LIST_TEXT_MAX ||
        to_length > JADREN_APP_LIST_TEXT_MAX ||
        !app_table_column_name_valid((const unsigned char *)from_data, from_length) ||
        !app_table_column_name_valid((const unsigned char *)to_data, to_length) ||
        jadren_app_table_migration.operation_count >= JADREN_APP_TABLE_MIGRATION_MAX_OPS) return 0;
    operation = &jadren_app_table_migration.operations[jadren_app_table_migration.operation_count];
    operation->kind = 1;
    operation->first_length = from_length;
    operation->second_length = to_length;
    operation->column_kind = 0;
    for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1) {
        operation->first[byte_index] = byte_index < from_length ? (unsigned char)from_data[byte_index] : 0;
        operation->second[byte_index] = byte_index < to_length ? (unsigned char)to_data[byte_index] : 0;
    }
    jadren_app_table_migration.operation_count += 1;
    return 1;
}

int app_table_migration_set_column_type(const char *name_data,
                                        unsigned __int64 name_length,
                                        int kind) {
    JadrenAppTableMigrationOp *operation;
    unsigned __int64 byte_index;
    if (!jadren_app_table_migration.active || name_data == 0 || name_length == 0 ||
        name_length > JADREN_APP_LIST_TEXT_MAX || kind < JADREN_APP_TABLE_TEXT ||
        kind > JADREN_APP_TABLE_FLOAT ||
        !app_table_column_name_valid((const unsigned char *)name_data, name_length) ||
        jadren_app_table_migration.operation_count >= JADREN_APP_TABLE_MIGRATION_MAX_OPS) return 0;
    operation = &jadren_app_table_migration.operations[jadren_app_table_migration.operation_count];
    operation->kind = 2;
    operation->first_length = name_length;
    operation->second_length = 0;
    operation->column_kind = kind;
    for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1)
        operation->first[byte_index] = byte_index < name_length ? (unsigned char)name_data[byte_index] : 0;
    jadren_app_table_migration.operation_count += 1;
    return 1;
}

int app_table_migration_commit(void) {
    int operation_index;
    int table_id;
    if (!jadren_app_table_migration.active) return 0;
    table_id = jadren_app_table_migration.table_id;
    if (!app_table_valid(table_id) ||
        jadren_app_tables[table_id].schema_version != jadren_app_table_migration.expected_version) {
        (void)app_table_tx_rollback();
        app_table_migration_reset();
        return 0;
    }
    for (operation_index = 0; operation_index < jadren_app_table_migration.operation_count; operation_index += 1) {
        JadrenAppTableMigrationOp *operation = &jadren_app_table_migration.operations[operation_index];
        int column_index = app_table_find_column(table_id, (const char *)operation->first,
                                                 operation->first_length);
        if (column_index < 0 ||
            (operation->kind == 1 && !app_table_set_column_name(
                table_id, column_index, (const char *)operation->second, operation->second_length)) ||
            (operation->kind == 2 && !app_table_set_column_type(
                table_id, column_index, operation->column_kind))) {
            (void)app_table_tx_rollback();
            app_table_migration_reset();
            return 0;
        }
    }
    if (!app_table_set_schema_version(table_id, jadren_app_table_migration.target_version) ||
        !app_table_tx_commit()) {
        (void)app_table_tx_rollback();
        app_table_migration_reset();
        return 0;
    }
    app_table_migration_reset();
    return 1;
}

int app_table_migration_rollback(void) {
    int result;
    if (!jadren_app_table_migration.active) return 0;
    result = app_table_tx_rollback();
    app_table_migration_reset();
    return result;
}

int app_table_sort_text(int table_id, int column_index, unsigned char descending) {
    int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS) {
        return 0;
    }
    for (row_index = 1; row_index < (int)jadren_app_tables[table_id].row_count; row_index += 1) {
        int cursor = row_index;
        while (cursor > 0) {
            int comparison = app_table_compare_items(
                &jadren_app_tables[table_id].cells[cursor - 1][column_index],
                &jadren_app_tables[table_id].cells[cursor][column_index]);
            if (descending) comparison = -comparison;
            if (comparison <= 0) break;
            app_table_swap_rows(&jadren_app_tables[table_id],
                                (unsigned int)(cursor - 1),
                                (unsigned int)cursor);
            cursor -= 1;
        }
    }
    return 1;
}

int app_table_sort_int(int table_id, int column_index, unsigned char descending) {
    int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_INT) return 0;
    for (row_index = 1; row_index < (int)jadren_app_tables[table_id].row_count; row_index += 1) {
        int cursor = row_index;
        while (cursor > 0) {
            const JadrenAppListItem *left = &jadren_app_tables[table_id].cells[cursor - 1][column_index];
            const JadrenAppListItem *right = &jadren_app_tables[table_id].cells[cursor][column_index];
            int comparison;
            if (left->length == 0 && right->length != 0) comparison = -1;
            else if (left->length != 0 && right->length == 0) comparison = 1;
            else {
                long long left_value = app_table_read_int(table_id, cursor - 1, column_index);
                long long right_value = app_table_read_int(table_id, cursor, column_index);
                comparison = left_value < right_value ? -1 : (left_value > right_value ? 1 : 0);
            }
            if (descending) comparison = -comparison;
            if (comparison <= 0) break;
            app_table_swap_rows(&jadren_app_tables[table_id], (unsigned int)(cursor - 1), (unsigned int)cursor);
            cursor -= 1;
        }
    }
    return 1;
}

int app_table_sort_uint(int table_id, int column_index, unsigned char descending) {
    int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_UINT) return 0;
    for (row_index = 1; row_index < (int)jadren_app_tables[table_id].row_count; row_index += 1) {
        int cursor = row_index;
        while (cursor > 0) {
            const JadrenAppListItem *left = &jadren_app_tables[table_id].cells[cursor - 1][column_index];
            const JadrenAppListItem *right = &jadren_app_tables[table_id].cells[cursor][column_index];
            int comparison;
            if (left->length == 0 && right->length != 0) comparison = -1;
            else if (left->length != 0 && right->length == 0) comparison = 1;
            else {
                unsigned __int64 left_value = app_table_read_uint(table_id, cursor - 1, column_index);
                unsigned __int64 right_value = app_table_read_uint(table_id, cursor, column_index);
                comparison = left_value < right_value ? -1 : (left_value > right_value ? 1 : 0);
            }
            if (descending) comparison = -comparison;
            if (comparison <= 0) break;
            app_table_swap_rows(&jadren_app_tables[table_id], (unsigned int)(cursor - 1), (unsigned int)cursor);
            cursor -= 1;
        }
    }
    return 1;
}

int app_table_sort_float(int table_id, int column_index, unsigned char descending) {
    int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_FLOAT) return 0;
    for (row_index = 1; row_index < (int)jadren_app_tables[table_id].row_count; row_index += 1) {
        int cursor = row_index;
        while (cursor > 0) {
            const JadrenAppListItem *left = &jadren_app_tables[table_id].cells[cursor - 1][column_index];
            const JadrenAppListItem *right = &jadren_app_tables[table_id].cells[cursor][column_index];
            int comparison;
            if (left->length == 0 && right->length != 0) comparison = -1;
            else if (left->length != 0 && right->length == 0) comparison = 1;
            else {
                double left_value = app_table_read_float(table_id, cursor - 1, column_index);
                double right_value = app_table_read_float(table_id, cursor, column_index);
                comparison = left_value < right_value ? -1 : (left_value > right_value ? 1 : 0);
            }
            if (descending) comparison = -comparison;
            if (comparison <= 0) break;
            app_table_swap_rows(&jadren_app_tables[table_id], (unsigned int)(cursor - 1), (unsigned int)cursor);
            cursor -= 1;
        }
    }
    return 1;
}

int app_table_sort_bool(int table_id, int column_index, unsigned char descending) {
    int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_BOOL) return 0;
    for (row_index = 1; row_index < (int)jadren_app_tables[table_id].row_count; row_index += 1) {
        int cursor = row_index;
        while (cursor > 0) {
            const JadrenAppListItem *left = &jadren_app_tables[table_id].cells[cursor - 1][column_index];
            const JadrenAppListItem *right = &jadren_app_tables[table_id].cells[cursor][column_index];
            int comparison;
            if (left->length == 0 && right->length != 0) comparison = -1;
            else if (left->length != 0 && right->length == 0) comparison = 1;
            else {
                int left_value = app_table_read_bool(table_id, cursor - 1, column_index);
                int right_value = app_table_read_bool(table_id, cursor, column_index);
                comparison = left_value < right_value ? -1 : (left_value > right_value ? 1 : 0);
            }
            if (descending) comparison = -comparison;
            if (comparison <= 0) break;
            app_table_swap_rows(&jadren_app_tables[table_id], (unsigned int)(cursor - 1), (unsigned int)cursor);
            cursor -= 1;
        }
    }
    return 1;
}

int app_table_sort_text_if_revision(int table_id, int column_index,
                                      unsigned char descending,
                                      unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_sort_text(table_id, column_index, descending);
}

int app_table_sort_int_if_revision(int table_id, int column_index,
                                   unsigned char descending,
                                   unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_sort_int(table_id, column_index, descending);
}

int app_table_sort_uint_if_revision(int table_id, int column_index,
                                    unsigned char descending,
                                    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_sort_uint(table_id, column_index, descending);
}

int app_table_sort_float_if_revision(int table_id, int column_index,
                                     unsigned char descending,
                                     unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_sort_float(table_id, column_index, descending);
}

int app_table_sort_bool_if_revision(int table_id, int column_index,
                                    unsigned char descending,
                                    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_sort_bool(table_id, column_index, descending);
}


int app_table_sort_callback(int table_id, int (*comparator)(int, int, int)) {
    JadrenAppTable sorted;
    unsigned char rows[JADREN_APP_TABLE_MAX_ROWS];
    unsigned int row_count;
    unsigned int row_index;
    unsigned int source_fingerprint;
    if (!app_table_valid(table_id) || comparator == 0 ||
        !app_table_store_valid(&jadren_app_tables[table_id])) return 0;
    row_count = (unsigned int)jadren_app_tables[table_id].row_count;
    source_fingerprint = app_table_index_fingerprint(&jadren_app_tables[table_id]);
    app_table_copy_store(&sorted, &jadren_app_tables[table_id]);
    for (row_index = 0; row_index < row_count; row_index += 1)
        rows[row_index] = (unsigned char)row_index;
    for (row_index = 1; row_index < row_count; row_index += 1) {
        unsigned char current = rows[row_index];
        unsigned int cursor = row_index;
        while (cursor > 0) {
            unsigned char previous = rows[cursor - 1];
            int comparison;
            if (app_table_index_fingerprint(&jadren_app_tables[table_id]) !=
                source_fingerprint) return 0;
            comparison = comparator(table_id, (int)previous, (int)current);
            if (app_table_index_fingerprint(&jadren_app_tables[table_id]) !=
                source_fingerprint) return 0;
            if (comparison <= 0) break;
            rows[cursor] = previous;
            cursor -= 1;
        }
        rows[cursor] = current;
    }
    for (row_index = 0; row_index < row_count; row_index += 1)
        app_table_copy_row(&sorted, row_index, &jadren_app_tables[table_id], rows[row_index]);
    if (app_table_index_fingerprint(&jadren_app_tables[table_id]) != source_fingerprint) return 0;
    app_table_copy_store(&jadren_app_tables[table_id], &sorted);
    return 1;
}

/* Sort through a caller-owned comparator only when the equality-only model
 * revision is still current. The unguarded path keeps its fingerprint checks
 * around every comparator call and publishes only after the complete scan. */
int app_table_sort_callback_if_revision(
    int table_id, int (*comparator)(int, int, int),
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_sort_callback(table_id, comparator);
}

int app_table_find_text(int table_id, int column_index, const char *query_data,
                        unsigned __int64 query_length, int start_row) {
    unsigned int row_index;
    unsigned __int64 byte_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS || start_row < 0 ||
        (unsigned __int64)start_row > jadren_app_tables[table_id].row_count ||
        query_length > JADREN_APP_LIST_TEXT_MAX ||
        (query_data == 0 && query_length > 0)) {
        return -1;
    }
    for (row_index = (unsigned int)start_row;
         row_index < jadren_app_tables[table_id].row_count; row_index += 1) {
        JadrenAppListItem *cell = &jadren_app_tables[table_id].cells[row_index][column_index];
        if (cell->length != query_length) continue;
        for (byte_index = 0; byte_index < query_length; byte_index += 1) {
            if (cell->text[byte_index] != (unsigned char)query_data[byte_index]) break;
        }
        if (byte_index == query_length) return (int)row_index;
    }
    return -1;
}

int app_table_find_int(int table_id, int column_index, long long query,
                       int start_row) {
    unsigned int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS || start_row < 0 ||
        (unsigned __int64)start_row > jadren_app_tables[table_id].row_count ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_INT) return -1;
    for (row_index = (unsigned int)start_row;
         row_index < jadren_app_tables[table_id].row_count; row_index += 1) {
        if (jadren_app_tables[table_id].cells[row_index][column_index].length == 0) continue;
        if (app_table_read_int(table_id, (int)row_index, column_index) == query) return (int)row_index;
    }
    return -1;
}

int app_table_find_uint(int table_id, int column_index, unsigned __int64 query,
                        int start_row) {
    unsigned int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS || start_row < 0 ||
        (unsigned __int64)start_row > jadren_app_tables[table_id].row_count ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_UINT) return -1;
    for (row_index = (unsigned int)start_row;
         row_index < jadren_app_tables[table_id].row_count; row_index += 1) {
        if (jadren_app_tables[table_id].cells[row_index][column_index].length == 0) continue;
        if (app_table_read_uint(table_id, (int)row_index, column_index) == query) return (int)row_index;
    }
    return -1;
}

int app_table_find_float(int table_id, int column_index, double query,
                         int start_row) {
    unsigned int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS || start_row < 0 ||
        (unsigned __int64)start_row > jadren_app_tables[table_id].row_count ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_FLOAT) return -1;
    for (row_index = (unsigned int)start_row;
         row_index < jadren_app_tables[table_id].row_count; row_index += 1) {
        if (jadren_app_tables[table_id].cells[row_index][column_index].length == 0) continue;
        if (app_table_read_float(table_id, (int)row_index, column_index) == query) return (int)row_index;
    }
    return -1;
}

int app_table_find_bool(int table_id, int column_index, unsigned char query,
                        int start_row) {
    unsigned int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS || start_row < 0 ||
        (unsigned __int64)start_row > jadren_app_tables[table_id].row_count ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_BOOL) return -1;
    for (row_index = (unsigned int)start_row;
         row_index < jadren_app_tables[table_id].row_count; row_index += 1) {
        if (jadren_app_tables[table_id].cells[row_index][column_index].length == 0) continue;
        if (app_table_read_bool(table_id, (int)row_index, column_index) == (query ? 1 : 0)) return (int)row_index;
    }
    return -1;
}

int app_table_remove_text(int table_id, int column_index, const char *key_data,
                          unsigned __int64 key_length) {
    int row_index = app_table_find_text(table_id, column_index, key_data, key_length, 0);
    if (row_index < 0) return 0;
    return app_table_remove_row(table_id, row_index);
}

int app_table_remove_int(int table_id, int column_index, long long key) {
    int row_index = app_table_find_int(table_id, column_index, key, 0);
    if (row_index < 0) return 0;
    return app_table_remove_row(table_id, row_index);
}

int app_table_remove_uint(int table_id, int column_index, unsigned __int64 key) {
    int row_index = app_table_find_uint(table_id, column_index, key, 0);
    if (row_index < 0) return 0;
    return app_table_remove_row(table_id, row_index);
}

int app_table_remove_float(int table_id, int column_index, double key) {
    int row_index = app_table_find_float(table_id, column_index, key, 0);
    if (row_index < 0) return 0;
    return app_table_remove_row(table_id, row_index);
}

int app_table_remove_bool(int table_id, int column_index, unsigned char key) {
    int row_index = app_table_find_bool(table_id, column_index, key, 0);
    if (row_index < 0) return 0;
    return app_table_remove_row(table_id, row_index);
}

int app_table_upsert_text(int table_id, int column_index, const char *key_data,
                          unsigned __int64 key_length) {
    int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_TEXT ||
        key_data == 0 || key_length == 0 || key_length > JADREN_APP_LIST_TEXT_MAX) return -1;
    row_index = app_table_find_text(table_id, column_index, key_data, key_length, 0);
    if (row_index >= 0) return row_index;
    if (!app_table_append_row(table_id)) return -1;
    row_index = (int)jadren_app_tables[table_id].row_count - 1;
    if (!app_table_set_cell(table_id, row_index, column_index, key_data, key_length)) {
        app_table_remove_row(table_id, row_index);
        return -1;
    }
    return row_index;
}

int app_table_upsert_int(int table_id, int column_index, long long key) {
    int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_INT) return -1;
    row_index = app_table_find_int(table_id, column_index, key, 0);
    if (row_index >= 0) return row_index;
    if (!app_table_append_row(table_id)) return -1;
    row_index = (int)jadren_app_tables[table_id].row_count - 1;
    if (!app_table_set_int(table_id, row_index, column_index, key)) {
        app_table_remove_row(table_id, row_index);
        return -1;
    }
    return row_index;
}

int app_table_upsert_uint(int table_id, int column_index, unsigned __int64 key) {
    int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_UINT) return -1;
    row_index = app_table_find_uint(table_id, column_index, key, 0);
    if (row_index >= 0) return row_index;
    if (!app_table_append_row(table_id)) return -1;
    row_index = (int)jadren_app_tables[table_id].row_count - 1;
    if (!app_table_set_uint(table_id, row_index, column_index, key)) {
        app_table_remove_row(table_id, row_index);
        return -1;
    }
    return row_index;
}

int app_table_upsert_float(int table_id, int column_index, double key) {
    int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_FLOAT) return -1;
    row_index = app_table_find_float(table_id, column_index, key, 0);
    if (row_index >= 0) return row_index;
    if (!app_table_append_row(table_id)) return -1;
    row_index = (int)jadren_app_tables[table_id].row_count - 1;
    if (!app_table_set_float(table_id, row_index, column_index, key)) {
        app_table_remove_row(table_id, row_index);
        return -1;
    }
    return row_index;
}

int app_table_upsert_bool(int table_id, int column_index, unsigned char key) {
    int row_index;
    if (!app_table_valid(table_id) || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[table_id].column_types[column_index] != JADREN_APP_TABLE_BOOL) return -1;
    row_index = app_table_find_bool(table_id, column_index, key, 0);
    if (row_index >= 0) return row_index;
    if (!app_table_append_row(table_id)) return -1;
    row_index = (int)jadren_app_tables[table_id].row_count - 1;
    if (!app_table_set_bool(table_id, row_index, column_index, key)) {
        app_table_remove_row(table_id, row_index);
        return -1;
    }
    return row_index;
}

int app_table_index_find_text(int table_id, int column_index,
                              const char *query_data,
                              unsigned __int64 query_length) {
    JadrenAppTableIndex *index;
    JadrenAppListItem query;
    int low;
    int high;
    if (!app_table_index_matches(table_id, column_index, JADREN_APP_TABLE_TEXT) ||
        (query_data == 0 && query_length > 0) ||
        query_length > JADREN_APP_LIST_TEXT_MAX) return -1;
    index = &jadren_app_table_indexes[table_id];
    query.length = query_length;
    for (unsigned __int64 byte_index = 0; byte_index < query_length; byte_index += 1)
        query.text[byte_index] = (unsigned char)query_data[byte_index];
    low = 0;
    high = (int)index->row_count;
    while (low < high) {
        int middle = low + (high - low) / 2;
        int row_index = (int)index->rows[middle];
        int comparison = app_table_compare_items(
            &jadren_app_tables[table_id].cells[row_index][column_index], &query);
        if (comparison < 0) low = middle + 1;
        else high = middle;
    }
    if (low >= (int)index->row_count) return -1;
    if (app_table_compare_items(
            &jadren_app_tables[table_id].cells[index->rows[low]][column_index],
            &query) != 0) return -1;
    return (int)index->rows[low];
}

int app_table_index_find_text_if_revision(
    int table_id, int column_index, const char *query_data,
    unsigned __int64 query_length, unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return -1;
    return app_table_index_find_text(table_id, column_index, query_data, query_length);
}

static int app_table_index_compare_query(int table_id, int column_index,
                                         unsigned int row_index, int kind,
                                         long long int_query,
                                         unsigned __int64 uint_query,
                                         double float_query,
                                         unsigned char bool_query) {
    JadrenAppListItem *cell = &jadren_app_tables[table_id].cells[row_index][column_index];
    if (cell->length == 0) return -1;
    if (kind == JADREN_APP_TABLE_INT) {
        long long value = app_table_read_int(table_id, (int)row_index, column_index);
        return value < int_query ? -1 : (value > int_query ? 1 : 0);
    }
    if (kind == JADREN_APP_TABLE_UINT) {
        unsigned __int64 value = app_table_read_uint(table_id, (int)row_index, column_index);
        return value < uint_query ? -1 : (value > uint_query ? 1 : 0);
    }
    if (kind == JADREN_APP_TABLE_FLOAT) {
        double value = app_table_read_float(table_id, (int)row_index, column_index);
        return value < float_query ? -1 : (value > float_query ? 1 : 0);
    }
    if (kind == JADREN_APP_TABLE_BOOL) {
        int value = app_table_read_bool(table_id, (int)row_index, column_index);
        return value < (bool_query ? 1 : 0) ? -1 :
               (value > (bool_query ? 1 : 0) ? 1 : 0);
    }
    return 1;
}

static int app_table_index_find_typed(int table_id, int column_index, int kind,
                                      long long int_query,
                                      unsigned __int64 uint_query,
                                      double float_query,
                                      unsigned char bool_query) {
    JadrenAppTableIndex *index;
    int low;
    int high;
    if (!app_table_index_matches(table_id, column_index, kind)) return -1;
    if (kind == JADREN_APP_TABLE_FLOAT && (float_query != float_query)) return -1;
    index = &jadren_app_table_indexes[table_id];
    low = 0;
    high = (int)index->row_count;
    while (low < high) {
        int middle = low + (high - low) / 2;
        int row_index = (int)index->rows[middle];
        int comparison = app_table_index_compare_query(
            table_id, column_index, (unsigned int)row_index, kind,
            int_query, uint_query, float_query, bool_query);
        if (comparison < 0) low = middle + 1;
        else high = middle;
    }
    if (low >= (int)index->row_count) return -1;
    if (app_table_index_compare_query(
            table_id, column_index, index->rows[low], kind,
            int_query, uint_query, float_query, bool_query) != 0) return -1;
    return (int)index->rows[low];
}

int app_table_index_find_int(int table_id, int column_index, long long query) {
    return app_table_index_find_typed(table_id, column_index, JADREN_APP_TABLE_INT,
                                      query, 0, 0.0, 0);
}

unsigned __int64 app_table_index_collect_int_range(int table_id, int column_index,
                                                   long long lower, long long upper,
                                                   int *output_data,
                                                   unsigned __int64 output_length) {
    JadrenAppTableIndex *index;
    unsigned int low;
    unsigned int high;
    unsigned int start;
    unsigned int end;
    unsigned int cursor;
    unsigned int count;
    if (lower > upper || !app_table_index_matches(table_id, column_index,
                                                   JADREN_APP_TABLE_INT)) return 0;
    index = &jadren_app_table_indexes[table_id];
    low = 0;
    high = index->row_count;
    while (low < high) {
        unsigned int middle = low + (high - low) / 2;
        int comparison = app_table_index_compare_query(
            table_id, column_index, index->rows[middle], JADREN_APP_TABLE_INT,
            lower, 0, 0.0, 0);
        if (comparison < 0) low = middle + 1;
        else high = middle;
    }
    start = low;
    low = start;
    high = index->row_count;
    while (low < high) {
        unsigned int middle = low + (high - low) / 2;
        int comparison = app_table_index_compare_query(
            table_id, column_index, index->rows[middle], JADREN_APP_TABLE_INT,
            upper, 0, 0.0, 0);
        if (comparison <= 0) low = middle + 1;
        else high = middle;
    }
    end = low;
    count = end - start;
    if (count == 0) return 0;
    if (output_data == 0 || output_length < (unsigned __int64)count) return 0;
    for (cursor = start; cursor < end; cursor += 1)
        output_data[cursor - start] = (int)index->rows[cursor];
    return (unsigned __int64)count;
}

unsigned __int64 app_table_index_collect_int_range_if_revision(
    int table_id, int column_index, long long lower, long long upper,
    int *output_data, unsigned __int64 output_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_index_collect_int_range(
        table_id, column_index, lower, upper, output_data, output_length);
}

static unsigned __int64 app_table_index_collect_typed_range(
    int table_id, int column_index, int kind,
    unsigned __int64 lower_uint, unsigned __int64 upper_uint,
    double lower_float, double upper_float,
    int *output_data, unsigned __int64 output_length) {
    JadrenAppTableIndex *index;
    unsigned int low;
    unsigned int high;
    unsigned int start;
    unsigned int end;
    unsigned int cursor;
    unsigned int count;
    if (!app_table_index_matches(table_id, column_index, kind)) return 0;
    if (kind == JADREN_APP_TABLE_UINT && lower_uint > upper_uint) return 0;
    if (kind == JADREN_APP_TABLE_FLOAT &&
        ((lower_float != lower_float) || (upper_float != upper_float) ||
         lower_float > 1.7976931348623157e308 ||
         lower_float < -1.7976931348623157e308 ||
         upper_float > 1.7976931348623157e308 ||
         upper_float < -1.7976931348623157e308 ||
         lower_float > upper_float)) return 0;
    index = &jadren_app_table_indexes[table_id];
    low = 0;
    high = index->row_count;
    while (low < high) {
        unsigned int middle = low + (high - low) / 2;
        int comparison = app_table_index_compare_query(
            table_id, column_index, index->rows[middle], kind,
            0, lower_uint, lower_float, 0);
        if (comparison < 0) low = middle + 1;
        else high = middle;
    }
    start = low;
    low = start;
    high = index->row_count;
    while (low < high) {
        unsigned int middle = low + (high - low) / 2;
        int comparison = app_table_index_compare_query(
            table_id, column_index, index->rows[middle], kind,
            0, upper_uint, upper_float, 0);
        if (comparison <= 0) low = middle + 1;
        else high = middle;
    }
    end = low;
    count = end - start;
    if (count == 0) return 0;
    if (output_data == 0 || output_length < (unsigned __int64)count) return 0;
    for (cursor = start; cursor < end; cursor += 1)
        output_data[cursor - start] = (int)index->rows[cursor];
    return (unsigned __int64)count;
}

unsigned __int64 app_table_index_collect_uint_range(int table_id, int column_index,
                                                    unsigned __int64 lower,
                                                    unsigned __int64 upper,
                                                    int *output_data,
                                                    unsigned __int64 output_length) {
    return app_table_index_collect_typed_range(
        table_id, column_index, JADREN_APP_TABLE_UINT,
        lower, upper, 0.0, 0.0, output_data, output_length);
}

unsigned __int64 app_table_index_collect_float_range(int table_id, int column_index,
                                                     double lower, double upper,
                                                     int *output_data,
                                                     unsigned __int64 output_length) {
    return app_table_index_collect_typed_range(
        table_id, column_index, JADREN_APP_TABLE_FLOAT,
        0, 0, lower, upper, output_data, output_length);
}

unsigned __int64 app_table_index_collect_uint_range_if_revision(
    int table_id, int column_index, unsigned __int64 lower,
    unsigned __int64 upper, int *output_data, unsigned __int64 output_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_index_collect_uint_range(
        table_id, column_index, lower, upper, output_data, output_length);
}

unsigned __int64 app_table_index_collect_float_range_if_revision(
    int table_id, int column_index, double lower, double upper,
    int *output_data, unsigned __int64 output_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_index_collect_float_range(
        table_id, column_index, lower, upper, output_data, output_length);
}

int app_table_index_find_uint(int table_id, int column_index, unsigned __int64 query) {
    return app_table_index_find_typed(table_id, column_index, JADREN_APP_TABLE_UINT,
                                      0, query, 0.0, 0);
}

int app_table_index_find_float(int table_id, int column_index, double query) {
    return app_table_index_find_typed(table_id, column_index, JADREN_APP_TABLE_FLOAT,
                                      0, 0, query, 0);
}

int app_table_index_find_bool(int table_id, int column_index, unsigned char query) {
    return app_table_index_find_typed(table_id, column_index, JADREN_APP_TABLE_BOOL,
                                      0, 0, 0.0, query);
}

int app_table_index_find_int_if_revision(
    int table_id, int column_index, long long query,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return -1;
    return app_table_index_find_int(table_id, column_index, query);
}

int app_table_index_find_uint_if_revision(
    int table_id, int column_index, unsigned __int64 query,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return -1;
    return app_table_index_find_uint(table_id, column_index, query);
}

int app_table_index_find_float_if_revision(
    int table_id, int column_index, double query,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return -1;
    return app_table_index_find_float(table_id, column_index, query);
}

int app_table_index_find_bool_if_revision(
    int table_id, int column_index, unsigned char query,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return -1;
    return app_table_index_find_bool(table_id, column_index, query);
}

static unsigned char app_table_filter_fold_ascii(unsigned char value,
                                                  int case_insensitive) {
    if (case_insensitive && value >= (unsigned char)'A' && value <= (unsigned char)'Z') {
        return (unsigned char)(value + ((unsigned char)'a' - (unsigned char)'A'));
    }
    return value;
}

static int app_table_filter_matches(const JadrenAppListItem *cell,
                                    const char *query_data,
                                    unsigned __int64 query_length,
                                    int mode) {
    unsigned __int64 start;
    unsigned __int64 byte_index;
    int case_insensitive = mode >= 4;
    if (case_insensitive) mode -= 4;
    if (mode < 0 || mode > 3 || cell == 0 || (query_data == 0 && query_length > 0)) {
        return 0;
    }
    if (mode == 0 && cell->length != query_length) return 0;
    if (mode != 0 && query_length > cell->length) return 0;
    if (mode == 0 || mode == 2) {
        start = 0;
    } else if (mode == 3) {
        start = cell->length - query_length;
    } else {
        for (start = 0; start + query_length <= cell->length; start += 1) {
            for (byte_index = 0; byte_index < query_length; byte_index += 1) {
                if (app_table_filter_fold_ascii(cell->text[start + byte_index], case_insensitive) !=
                    app_table_filter_fold_ascii((unsigned char)query_data[byte_index], case_insensitive)) {
                    break;
                }
            }
            if (byte_index == query_length) return 1;
        }
        return 0;
    }
    for (byte_index = 0; byte_index < query_length; byte_index += 1) {
        if (app_table_filter_fold_ascii(cell->text[start + byte_index], case_insensitive) !=
            app_table_filter_fold_ascii((unsigned char)query_data[byte_index], case_insensitive)) {
            return 0;
        }
    }
    return 1;
}

int app_table_filter_text_ex(int source_table_id, int destination_table_id,
                             int column_index, const char *query_data,
                             unsigned __int64 query_length, int mode) {
    unsigned int source_row;
    if (!app_table_valid(source_table_id) || !app_table_valid(destination_table_id) ||
        source_table_id == destination_table_id || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        query_length > JADREN_APP_LIST_TEXT_MAX ||
        (query_data == 0 && query_length > 0) || mode < 0 || mode > 7) {
        return 0;
    }
    if (!app_table_store_valid(&jadren_app_tables[source_table_id])) return 0;
    app_table_copy_schema(&jadren_app_tables[destination_table_id],
                          &jadren_app_tables[source_table_id]);
    app_table_clear(destination_table_id);
    for (source_row = 0;
         source_row < jadren_app_tables[source_table_id].row_count; source_row += 1) {
        JadrenAppListItem *cell =
            &jadren_app_tables[source_table_id].cells[source_row][column_index];
        if (!app_table_filter_matches(cell, query_data, query_length, mode)) continue;
        if (jadren_app_tables[destination_table_id].row_count >= JADREN_APP_TABLE_MAX_ROWS) {
            return 0;
        }
        app_table_copy_row(
            &jadren_app_tables[destination_table_id],
            (unsigned int)jadren_app_tables[destination_table_id].row_count,
            &jadren_app_tables[source_table_id], source_row);
        jadren_app_tables[destination_table_id].row_count += 1;
    }
    return 1;
}

int app_table_filter_text_ex_bytes(int source_table_id, int destination_table_id,
                                   int column_index, const unsigned char *query_data,
                                   unsigned __int64 query_capacity,
                                   unsigned __int64 query_length, int mode) {
    if (query_length > query_capacity) return 0;
    return app_table_filter_text_ex(source_table_id, destination_table_id,
                                    column_index, (const char *)query_data,
                                    query_length, mode);
}

int app_table_filter_text(int source_table_id, int destination_table_id,
                          int column_index, const char *query_data,
                          unsigned __int64 query_length) {
    return app_table_filter_text_ex(source_table_id, destination_table_id,
                                    column_index, query_data, query_length, 0);
}

/* Filter one text column only when the caller's equality-only model revision
 * is still current. A stale revision leaves both tables unchanged; this is
 * process-local coordination, not a cross-thread atomic or persistence
 * primitive. */
int app_table_filter_text_if_revision(int source_table_id, int destination_table_id,
                                      int column_index, const char *query_data,
                                      unsigned __int64 query_length,
                                      unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_filter_text(source_table_id, destination_table_id,
                                 column_index, query_data, query_length);
}

/* Filter one text column with an explicit match mode only when the caller's
 * equality-only model revision is still current. */
int app_table_filter_text_ex_if_revision(int source_table_id, int destination_table_id,
                                         int column_index, const char *query_data,
                                         unsigned __int64 query_length, int mode,
                                         unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_filter_text_ex(source_table_id, destination_table_id,
                                    column_index, query_data, query_length, mode);
}

int app_table_filter_callback(int source_table_id, int destination_table_id,
                              unsigned char (*predicate)(int, int)) {
    JadrenAppTable filtered;
    unsigned int source_row;
    unsigned int source_fingerprint;
    if (!app_table_valid(source_table_id) || !app_table_valid(destination_table_id) ||
        source_table_id == destination_table_id || predicate == 0 ||
        !app_table_store_valid(&jadren_app_tables[source_table_id])) return 0;
    source_fingerprint = app_table_index_fingerprint(&jadren_app_tables[source_table_id]);
    app_table_copy_store(&filtered, &jadren_app_tables[source_table_id]);
    filtered.row_count = 0;
    for (source_row = 0; source_row < jadren_app_tables[source_table_id].row_count;
         source_row += 1) {
        if (app_table_index_fingerprint(&jadren_app_tables[source_table_id]) !=
            source_fingerprint) return 0;
        if (!predicate(source_table_id, (int)source_row)) continue;
        if (filtered.row_count >= JADREN_APP_TABLE_MAX_ROWS) return 0;
        app_table_copy_row(&filtered, (unsigned int)filtered.row_count,
                           &jadren_app_tables[source_table_id], source_row);
        filtered.row_count += 1;
    }
    if (app_table_index_fingerprint(&jadren_app_tables[source_table_id]) !=
        source_fingerprint) return 0;
    app_table_copy_store(&jadren_app_tables[destination_table_id], &filtered);
    return 1;
}

/* Apply a callback filter only when the caller's equality-only model revision
 * is still current. The unguarded implementation retains its source
 * fingerprint checks while this entry point rejects a stale token before the
 * scan starts. */
int app_table_filter_callback_if_revision(
    int source_table_id, int destination_table_id,
    unsigned char (*predicate)(int, int), unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_filter_callback(source_table_id, destination_table_id, predicate);
}

int app_table_page(int source_table_id, int destination_table_id,
                   int start_row, int page_size) {
    JadrenAppTable page;
    unsigned int source_count;
    unsigned int requested;
    unsigned int row_index;
    if (!app_table_valid(source_table_id) || !app_table_valid(destination_table_id) ||
        source_table_id == destination_table_id || start_row < 0 || page_size < 0 ||
        page_size > JADREN_APP_TABLE_MAX_ROWS ||
        !app_table_store_valid(&jadren_app_tables[source_table_id])) return 0;
    source_count = (unsigned int)jadren_app_tables[source_table_id].row_count;
    if ((unsigned int)start_row > source_count) return 0;
    requested = (unsigned int)page_size;
    if (requested > source_count - (unsigned int)start_row)
        requested = source_count - (unsigned int)start_row;
    app_table_copy_store(&page, &jadren_app_tables[source_table_id]);
    page.row_count = 0;
    for (row_index = 0; row_index < requested; row_index += 1) {
        app_table_copy_row(&page, row_index, &jadren_app_tables[source_table_id],
                           (unsigned int)start_row + row_index);
    }
    page.row_count = requested;
    app_table_copy_store(&jadren_app_tables[destination_table_id], &page);
    return 1;
}

/* Project one bounded table page only when the caller's equality-only model
 * revision is still current. A stale revision leaves both source and
 * destination tables unchanged; this is process-local coordination, not a
 * cross-thread atomic or persistence primitive. */
int app_table_page_if_revision(int source_table_id, int destination_table_id,
                               int start_row, int page_size,
                               unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_page(source_table_id, destination_table_id, start_row,
                          page_size);
}

int app_table_filter_int(int source_table_id, int destination_table_id,
                         int column_index, long long query) {
    unsigned int source_row;
    if (!app_table_valid(source_table_id) || !app_table_valid(destination_table_id) ||
        source_table_id == destination_table_id || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[source_table_id].column_types[column_index] != JADREN_APP_TABLE_INT) return 0;
    app_table_copy_schema(&jadren_app_tables[destination_table_id],
                          &jadren_app_tables[source_table_id]);
    app_table_clear(destination_table_id);
    for (source_row = 0; source_row < jadren_app_tables[source_table_id].row_count; source_row += 1) {
        if (jadren_app_tables[source_table_id].cells[source_row][column_index].length == 0 ||
            app_table_read_int(source_table_id, (int)source_row, column_index) != query) continue;
        if (jadren_app_tables[destination_table_id].row_count >= JADREN_APP_TABLE_MAX_ROWS) return 0;
        app_table_copy_row(&jadren_app_tables[destination_table_id],
                           (unsigned int)jadren_app_tables[destination_table_id].row_count,
                           &jadren_app_tables[source_table_id], source_row);
        jadren_app_tables[destination_table_id].row_count += 1;
    }
    return 1;
}

int app_table_filter_int_if_revision(int source_table_id, int destination_table_id,
                                     int column_index, long long query,
                                     unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_filter_int(source_table_id, destination_table_id,
                                column_index, query);
}

int app_table_filter_uint(int source_table_id, int destination_table_id,
                          int column_index, unsigned __int64 query) {
    unsigned int source_row;
    if (!app_table_valid(source_table_id) || !app_table_valid(destination_table_id) ||
        source_table_id == destination_table_id || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[source_table_id].column_types[column_index] != JADREN_APP_TABLE_UINT) return 0;
    app_table_copy_schema(&jadren_app_tables[destination_table_id],
                          &jadren_app_tables[source_table_id]);
    app_table_clear(destination_table_id);
    for (source_row = 0; source_row < jadren_app_tables[source_table_id].row_count; source_row += 1) {
        if (jadren_app_tables[source_table_id].cells[source_row][column_index].length == 0 ||
            app_table_read_uint(source_table_id, (int)source_row, column_index) != query) continue;
        if (jadren_app_tables[destination_table_id].row_count >= JADREN_APP_TABLE_MAX_ROWS) return 0;
        app_table_copy_row(&jadren_app_tables[destination_table_id],
                           (unsigned int)jadren_app_tables[destination_table_id].row_count,
                           &jadren_app_tables[source_table_id], source_row);
        jadren_app_tables[destination_table_id].row_count += 1;
    }
    return 1;
}

int app_table_filter_uint_if_revision(int source_table_id, int destination_table_id,
                                      int column_index, unsigned __int64 query,
                                      unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_filter_uint(source_table_id, destination_table_id,
                                 column_index, query);
}

int app_table_filter_float(int source_table_id, int destination_table_id,
                           int column_index, double query) {
    unsigned int source_row;
    if (!app_table_valid(source_table_id) || !app_table_valid(destination_table_id) ||
        source_table_id == destination_table_id || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[source_table_id].column_types[column_index] != JADREN_APP_TABLE_FLOAT) return 0;
    app_table_copy_schema(&jadren_app_tables[destination_table_id],
                          &jadren_app_tables[source_table_id]);
    app_table_clear(destination_table_id);
    for (source_row = 0; source_row < jadren_app_tables[source_table_id].row_count; source_row += 1) {
        if (jadren_app_tables[source_table_id].cells[source_row][column_index].length == 0 ||
            app_table_read_float(source_table_id, (int)source_row, column_index) != query) continue;
        if (jadren_app_tables[destination_table_id].row_count >= JADREN_APP_TABLE_MAX_ROWS) return 0;
        app_table_copy_row(&jadren_app_tables[destination_table_id],
                           (unsigned int)jadren_app_tables[destination_table_id].row_count,
                           &jadren_app_tables[source_table_id], source_row);
        jadren_app_tables[destination_table_id].row_count += 1;
    }
    return 1;
}

int app_table_filter_float_if_revision(int source_table_id, int destination_table_id,
                                       int column_index, double query,
                                       unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_filter_float(source_table_id, destination_table_id,
                                  column_index, query);
}

int app_table_filter_bool(int source_table_id, int destination_table_id,
                          int column_index, unsigned char query) {
    unsigned int source_row;
    if (!app_table_valid(source_table_id) || !app_table_valid(destination_table_id) ||
        source_table_id == destination_table_id || column_index < 0 ||
        column_index >= JADREN_APP_TABLE_MAX_COLUMNS ||
        jadren_app_tables[source_table_id].column_types[column_index] != JADREN_APP_TABLE_BOOL) return 0;
    app_table_copy_schema(&jadren_app_tables[destination_table_id],
                          &jadren_app_tables[source_table_id]);
    app_table_clear(destination_table_id);
    for (source_row = 0; source_row < jadren_app_tables[source_table_id].row_count; source_row += 1) {
        if (jadren_app_tables[source_table_id].cells[source_row][column_index].length == 0 ||
            app_table_read_bool(source_table_id, (int)source_row, column_index) != (query ? 1 : 0)) continue;
        if (jadren_app_tables[destination_table_id].row_count >= JADREN_APP_TABLE_MAX_ROWS) return 0;
        app_table_copy_row(&jadren_app_tables[destination_table_id],
                           (unsigned int)jadren_app_tables[destination_table_id].row_count,
                           &jadren_app_tables[source_table_id], source_row);
        jadren_app_tables[destination_table_id].row_count += 1;
    }
    return 1;
}

int app_table_filter_bool_if_revision(int source_table_id, int destination_table_id,
                                      int column_index, unsigned char query,
                                      unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_filter_bool(source_table_id, destination_table_id,
                                 column_index, query);
}

#define JADREN_APP_TABLE_DOCUMENT_MAX 524288

int app_table_save(int table_id, const char *path_data, unsigned __int64 path_length) {
    unsigned char document[JADREN_APP_TABLE_DOCUMENT_MAX];
    unsigned __int64 offset = 0;
    unsigned int row_index;
    unsigned int column_index;
    if (!app_table_valid(table_id) || path_data == 0 ||
        !app_state_append_byte(document, sizeof(document), &offset, (unsigned char)'[')) {
        return 0;
    }
    for (row_index = 0; row_index < jadren_app_tables[table_id].row_count; row_index += 1) {
        if (row_index > 0 && !app_state_append_byte(document, sizeof(document), &offset,
                                                    (unsigned char)',')) {
            return 0;
        }
        if (!app_state_append_byte(document, sizeof(document), &offset, (unsigned char)'[')) {
            return 0;
        }
        for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
            JadrenAppListItem *cell = &jadren_app_tables[table_id].cells[row_index][column_index];
            if (column_index > 0 && !app_state_append_byte(document, sizeof(document), &offset,
                                                            (unsigned char)',')) {
                return 0;
            }
            if (!app_state_append_json_string(document, sizeof(document), &offset,
                                              cell->text, cell->length)) {
                return 0;
            }
        }
        if (!app_state_append_byte(document, sizeof(document), &offset, (unsigned char)']')) {
            return 0;
        }
    }
    if (!app_state_append_byte(document, sizeof(document), &offset, (unsigned char)']')) {
        return 0;
    }
    return file_write_text(path_data, path_length, (const char *)document, offset) == offset;
}

static unsigned __int64 app_table_csv_field_required(const JadrenAppListItem *cell) {
    unsigned __int64 required;
    unsigned __int64 index;
    int quoted;
    if (cell == 0) return 0;
    quoted = csv_escape_requires_quotes(cell->text, cell->length);
    required = quoted ? 2ULL : 0ULL;
    for (index = 0; index < cell->length; index += 1) {
        unsigned __int64 addition = cell->text[index] == '"' ? 2ULL : 1ULL;
        if (required > ((unsigned __int64)-1) - addition) return 0;
        required += addition;
    }
    return required;
}

static int app_table_csv_add_size(unsigned __int64 *total,
                                   unsigned __int64 value) {
    if (total == 0 || *total > ((unsigned __int64)-1) - value) return 0;
    *total += value;
    return 1;
}

static unsigned __int64 app_table_csv_field_write(
    const JadrenAppListItem *cell, unsigned char *output_data,
    unsigned __int64 output_length) {
    unsigned __int64 required;
    unsigned __int64 index;
    unsigned __int64 offset = 0;
    int quoted;
    if (cell == 0) return 0;
    required = app_table_csv_field_required(cell);
    if ((output_data == 0 && required > 0) || output_length < required) return 0;
    quoted = csv_escape_requires_quotes(cell->text, cell->length);
    if (quoted) output_data[offset++] = (unsigned char)'"';
    for (index = 0; index < cell->length; index += 1) {
        if (cell->text[index] == '"') output_data[offset++] = (unsigned char)'"';
        output_data[offset++] = cell->text[index];
    }
    if (quoted) output_data[offset++] = (unsigned char)'"';
    return offset;
}

unsigned __int64 app_list_export_csv(int list_id, unsigned char *output_data,
                                     unsigned __int64 output_length) {
    const JadrenAppList *list;
    unsigned int item_index;
    unsigned __int64 required = 0;
    unsigned __int64 written = 0;
    unsigned __int64 field_length;
    if (!app_list_valid(list_id) || output_data == 0) return 0;
    list = &jadren_app_lists[list_id];
    for (item_index = 0; item_index < list->count; item_index += 1) {
        field_length = app_table_csv_field_required(&list->items[item_index]);
        if (!app_table_csv_add_size(&required, field_length) ||
            !app_table_csv_add_size(&required, 1)) return 0;
    }
    if (output_length < required) return 0;
    for (item_index = 0; item_index < list->count; item_index += 1) {
        field_length = app_table_csv_field_required(&list->items[item_index]);
        if (field_length > 0) {
            if (app_table_csv_field_write(&list->items[item_index],
                                          output_data + written, output_length - written) != field_length) return 0;
            written += field_length;
        }
        output_data[written++] = (unsigned char)'\n';
    }
    return written == required ? written : 0;
}

int app_list_export_csv_exact(int list_id, unsigned char *output_data,
                              unsigned __int64 output_capacity,
                              unsigned __int64 *output_text_length,
                              unsigned __int64 output_text_length_capacity) {
    const JadrenAppList *list;
    unsigned __int64 written;
    if (!app_list_valid(list_id) || output_text_length == 0 ||
        output_text_length_capacity == 0) return 0;
    list = &jadren_app_lists[list_id];
    if (list->count == 0) {
        output_text_length[0] = 0;
        return 1;
    }
    if (output_data == 0) return 0;
    written = app_list_export_csv(list_id, output_data, output_capacity);
    if (written == 0) return 0;
    output_text_length[0] = written;
    return 1;
}

static int app_json_add_size(unsigned __int64 *total,
                             unsigned __int64 addition) {
    if (total == 0 || *total > ((unsigned __int64)-1) - addition) return 0;
    *total += addition;
    return 1;
}

int app_list_export_json_exact(int list_id, unsigned char *output_data,
                               unsigned __int64 output_capacity,
                               unsigned __int64 *output_text_length,
                               unsigned __int64 output_text_length_capacity) {
    const JadrenAppList *list;
    unsigned __int64 required = 2ULL;
    unsigned __int64 written = 0;
    unsigned __int64 field_length;
    unsigned int item_index;
    if (!app_list_valid(list_id) || output_text_length == 0 ||
        output_text_length_capacity == 0) return 0;
    list = &jadren_app_lists[list_id];
    for (item_index = 0; item_index < list->count; item_index += 1) {
        field_length = json_escaped_length(list->items[item_index].text,
                                            list->items[item_index].length);
        if (field_length == 0 || (item_index > 0 && !app_json_add_size(&required, 1)) ||
            !app_json_add_size(&required, field_length)) return 0;
    }
    if (output_data == 0 || output_capacity < required) return 0;
    output_data[written++] = (unsigned char)'[';
    for (item_index = 0; item_index < list->count; item_index += 1) {
        if (item_index > 0) output_data[written++] = (unsigned char)',';
        written = json_write_escaped(list->items[item_index].text,
                                     list->items[item_index].length,
                                     output_data, written);
    }
    output_data[written++] = (unsigned char)']';
    if (written != required) return 0;
    output_text_length[0] = written;
    return 1;
}

static unsigned int app_table_csv_column_count(const JadrenAppTable *table) {
    int last_column = -1;
    unsigned int column_index;
    unsigned int row_index;
    if (table == 0) return 0;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        if (table->column_names[column_index].length > 0) last_column = (int)column_index;
    }
    for (row_index = 0; row_index < table->row_count; row_index += 1) {
        for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
            if (table->cells[row_index][column_index].length > 0) last_column = (int)column_index;
        }
    }
    return last_column < 0 ? 0U : (unsigned int)(last_column + 1);
}

unsigned __int64 app_table_export_csv(int table_id, unsigned char *output_data,
                                      unsigned __int64 output_length) {
    const JadrenAppTable *table;
    unsigned int column_count;
    unsigned int row_index;
    unsigned int column_index;
    unsigned __int64 required = 0;
    unsigned __int64 written = 0;
    unsigned __int64 field_length;
    if (!app_table_valid(table_id) || output_data == 0) return 0;
    table = &jadren_app_tables[table_id];
    if (!app_table_store_valid(table)) return 0;
    column_count = app_table_csv_column_count(table);
    if (column_count == 0) return 0;
    for (column_index = 0; column_index < column_count; column_index += 1) {
        if (!app_table_csv_add_size(
                &required, app_table_csv_field_required(&table->column_names[column_index]))) return 0;
        if (column_index + 1 < column_count && !app_table_csv_add_size(&required, 1)) return 0;
    }
    if (!app_table_csv_add_size(&required, 1)) return 0;
    for (row_index = 0; row_index < table->row_count; row_index += 1) {
        for (column_index = 0; column_index < column_count; column_index += 1) {
            if (!app_table_csv_add_size(
                    &required, app_table_csv_field_required(&table->cells[row_index][column_index]))) return 0;
            if (column_index + 1 < column_count && !app_table_csv_add_size(&required, 1)) return 0;
        }
        if (!app_table_csv_add_size(&required, 1)) return 0;
    }
    if (output_length < required) return 0;
    for (column_index = 0; column_index < column_count; column_index += 1) {
        field_length = app_table_csv_field_required(&table->column_names[column_index]);
        if (field_length > 0) {
            if (app_table_csv_field_write(&table->column_names[column_index],
                                          output_data + written, output_length - written) != field_length) return 0;
            written += field_length;
        }
        if (column_index + 1 < column_count) output_data[written++] = (unsigned char)',';
    }
    output_data[written++] = (unsigned char)'\n';
    for (row_index = 0; row_index < table->row_count; row_index += 1) {
        for (column_index = 0; column_index < column_count; column_index += 1) {
            field_length = app_table_csv_field_required(&table->cells[row_index][column_index]);
            if (field_length > 0) {
                if (app_table_csv_field_write(&table->cells[row_index][column_index],
                                              output_data + written, output_length - written) != field_length) return 0;
                written += field_length;
            }
            if (column_index + 1 < column_count) output_data[written++] = (unsigned char)',';
        }
        output_data[written++] = (unsigned char)'\n';
    }
    return written == required ? written : 0;
}

int app_table_export_csv_exact(int table_id, unsigned char *output_data,
                               unsigned __int64 output_capacity,
                               unsigned __int64 *output_text_length,
                               unsigned __int64 output_text_length_capacity) {
    unsigned __int64 written;
    if (output_text_length == 0 || output_text_length_capacity == 0 ||
        output_data == 0) return 0;
    written = app_table_export_csv(table_id, output_data, output_capacity);
    if (written == 0) return 0;
    output_text_length[0] = written;
    return 1;
}

int app_table_export_json_exact(int table_id, unsigned char *output_data,
                                unsigned __int64 output_capacity,
                                unsigned __int64 *output_text_length,
                                unsigned __int64 output_text_length_capacity) {
    const JadrenAppTable *table;
    unsigned int column_count;
    unsigned int row_index;
    unsigned int column_index;
    unsigned __int64 required = sizeof("{\"columns\":[") - 1ULL;
    unsigned __int64 written = 0;
    unsigned __int64 field_length;
    unsigned int literal_index;
    if (!app_table_valid(table_id) || output_text_length == 0 ||
        output_text_length_capacity == 0) return 0;
    table = &jadren_app_tables[table_id];
    if (!app_table_store_valid(table)) return 0;
    column_count = app_table_csv_column_count(table);
    for (column_index = 0; column_index < column_count; column_index += 1) {
        field_length = json_escaped_length(
            table->column_names[column_index].text,
            table->column_names[column_index].length);
        if (field_length == 0 ||
            (column_index > 0 && !app_json_add_size(&required, 1)) ||
            !app_json_add_size(&required, field_length)) return 0;
    }
    if (!app_json_add_size(&required, sizeof("],\"rows\":[") - 1ULL)) return 0;
    for (row_index = 0; row_index < table->row_count; row_index += 1) {
        if (row_index > 0 && !app_json_add_size(&required, 1)) return 0;
        if (!app_json_add_size(&required, 1)) return 0;
        for (column_index = 0; column_index < column_count; column_index += 1) {
            field_length = json_escaped_length(
                table->cells[row_index][column_index].text,
                table->cells[row_index][column_index].length);
            if (field_length == 0 ||
                (column_index > 0 && !app_json_add_size(&required, 1)) ||
                !app_json_add_size(&required, field_length)) return 0;
        }
        if (!app_json_add_size(&required, 1)) return 0;
    }
    if (!app_json_add_size(&required, sizeof("]}") - 1ULL) ||
        output_data == 0 || output_capacity < required) return 0;
    for (literal_index = 0; literal_index < sizeof("{\"columns\":[") - 1U;
         literal_index += 1U) {
        output_data[written + literal_index] =
            (unsigned char)"{\"columns\":["[literal_index];
    }
    written += sizeof("{\"columns\":[") - 1ULL;
    for (column_index = 0; column_index < column_count; column_index += 1) {
        if (column_index > 0) output_data[written++] = (unsigned char)',';
        written = json_write_escaped(table->column_names[column_index].text,
                                     table->column_names[column_index].length,
                                     output_data, written);
    }
    for (literal_index = 0; literal_index < sizeof("],\"rows\":[") - 1U;
        literal_index += 1U) {
        output_data[written + literal_index] =
            (unsigned char)"],\"rows\":["[literal_index];
    }
    written += sizeof("],\"rows\":[") - 1ULL;
    for (row_index = 0; row_index < table->row_count; row_index += 1) {
        if (row_index > 0) output_data[written++] = (unsigned char)',';
        output_data[written++] = (unsigned char)'[';
        for (column_index = 0; column_index < column_count; column_index += 1) {
            if (column_index > 0) output_data[written++] = (unsigned char)',';
            written = json_write_escaped(table->cells[row_index][column_index].text,
                                         table->cells[row_index][column_index].length,
                                         output_data, written);
        }
        output_data[written++] = (unsigned char)']';
    }
    output_data[written++] = (unsigned char)']';
    output_data[written++] = (unsigned char)'}';
    if (written != required) return 0;
    output_text_length[0] = written;
    return 1;
}

enum {
    JADREN_APP_TABLE_CSV_FIELD_COMMA = 1,
    JADREN_APP_TABLE_CSV_FIELD_ROW = 2
};

static int app_table_csv_read_field(const unsigned char *input_data,
                                    unsigned __int64 input_length,
                                    unsigned __int64 *offset,
                                    JadrenAppListItem *field) {
    unsigned __int64 index;
    unsigned __int64 written = 0;
    int quoted = 0;
    int closed = 0;
    unsigned char value;
    if (input_data == 0 || offset == 0 || field == 0 || *offset > input_length) return 0;
    app_list_clear_item(field);
    index = *offset;
    if (index < input_length && input_data[index] == (unsigned char)'"') {
        quoted = 1;
        index += 1;
        while (index < input_length) {
            value = input_data[index++];
            if (value == (unsigned char)'"') {
                if (index < input_length && input_data[index] == (unsigned char)'"') {
                    if (written >= JADREN_APP_LIST_TEXT_MAX) return 0;
                    field->text[written++] = (unsigned char)'"';
                    index += 1;
                    continue;
                }
                closed = 1;
                break;
            }
            if (written >= JADREN_APP_LIST_TEXT_MAX) return 0;
            field->text[written++] = value;
        }
        if (!closed) return 0;
    } else {
        while (index < input_length && input_data[index] != (unsigned char)',' &&
               input_data[index] != (unsigned char)'\n' && input_data[index] != (unsigned char)'\r') {
            value = input_data[index++];
            if (value == (unsigned char)'"' || written >= JADREN_APP_LIST_TEXT_MAX) return 0;
            field->text[written++] = value;
        }
    }
    field->length = written;
    if (index == input_length) {
        *offset = index;
        return JADREN_APP_TABLE_CSV_FIELD_ROW;
    }
    value = input_data[index];
    if (value == (unsigned char)',') {
        *offset = index + 1;
        return JADREN_APP_TABLE_CSV_FIELD_COMMA;
    }
    if (value == (unsigned char)'\n') {
        *offset = index + 1;
        return JADREN_APP_TABLE_CSV_FIELD_ROW;
    }
    if (value == (unsigned char)'\r') {
        if (index + 1 >= input_length || input_data[index + 1] != (unsigned char)'\n') return 0;
        *offset = index + 2;
        return JADREN_APP_TABLE_CSV_FIELD_ROW;
    }
    if (quoted) return 0;
    return 0;
}

static void app_table_csv_copy_item(JadrenAppListItem *destination,
                                    const JadrenAppListItem *source) {
    unsigned __int64 index;
    if (destination == 0 || source == 0) return;
    destination->length = source->length;
    for (index = 0; index < JADREN_APP_LIST_TEXT_MAX; index += 1)
        destination->text[index] = source->text[index];
}

int app_table_import_csv(int table_id, const unsigned char *input_data,
                         unsigned __int64 input_capacity,
                         unsigned __int64 input_length) {
    JadrenAppTable parsed;
    JadrenAppListItem parsed_names[JADREN_APP_TABLE_MAX_COLUMNS];
    JadrenAppListItem row_cells[JADREN_APP_TABLE_MAX_COLUMNS];
    unsigned __int64 offset = 0;
    unsigned int column_count = 0;
    unsigned int column_index;
    unsigned int row_index;
    int delimiter;
    if (!app_table_valid(table_id) || input_data == 0 || input_length == 0 ||
        input_length > input_capacity ||
        input_length > JADREN_APP_TABLE_CSV_INPUT_MAX) return 0;
    for (;;) {
        if (column_count >= JADREN_APP_TABLE_MAX_COLUMNS) return 0;
        delimiter = app_table_csv_read_field(input_data, input_length, &offset,
                                             &parsed_names[column_count]);
        if (!delimiter || !app_table_column_name_valid(
                parsed_names[column_count].text, parsed_names[column_count].length)) return 0;
        if (parsed_names[column_count].length > 0)
            for (column_index = 0; column_index < column_count; column_index += 1)
                if (app_table_column_name_matches(&parsed_names[column_index],
                                                  parsed_names[column_count].text,
                                                  parsed_names[column_count].length)) return 0;
        column_count += 1;
        if (delimiter == JADREN_APP_TABLE_CSV_FIELD_ROW) break;
    }
    app_table_copy_schema(&parsed, &jadren_app_tables[table_id]);
    app_table_clear_store(&parsed);
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        app_list_clear_item(&parsed.column_names[column_index]);
        if (column_index < column_count)
            app_table_csv_copy_item(&parsed.column_names[column_index], &parsed_names[column_index]);
    }
    while (offset < input_length) {
        if (parsed.row_count >= JADREN_APP_TABLE_MAX_ROWS) return 0;
        row_index = (unsigned int)parsed.row_count;
        for (column_index = 0; column_index < column_count; column_index += 1) {
            delimiter = app_table_csv_read_field(input_data, input_length, &offset,
                                                 &row_cells[column_index]);
            if (!delimiter) return 0;
            if (column_index + 1 < column_count) {
                if (delimiter != JADREN_APP_TABLE_CSV_FIELD_COMMA) return 0;
            } else if (delimiter != JADREN_APP_TABLE_CSV_FIELD_ROW) return 0;
            app_table_csv_copy_item(&parsed.cells[row_index][column_index],
                                    &row_cells[column_index]);
        }
        parsed.row_count += 1;
    }
    if (!app_table_store_valid(&parsed)) return 0;
    app_table_copy_store(&jadren_app_tables[table_id], &parsed);
    return 1;
}

int app_table_import_csv_if_revision(int table_id, const unsigned char *input_data,
                                     unsigned __int64 input_capacity,
                                     unsigned __int64 input_length,
                                     unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_import_csv(table_id, input_data, input_capacity, input_length);
}

/* Import one bounded CSV table file transactionally. The file is copied into
 * fixed scratch storage, then parsed by the same no-partial-update routine as
 * caller-owned CSV input. */
int app_table_import_csv_file(int table_id, const char *path_data,
                              unsigned __int64 path_length) {
    static unsigned char document[JADREN_APP_TABLE_CSV_INPUT_MAX];
    unsigned __int64 file_length;
    if (!app_table_valid(table_id) || path_data == 0 || path_length == 0) return 0;
    file_length = file_size(path_data, path_length);
    if (file_length == 0 || file_length > sizeof(document)) return 0;
    if (file_read(path_data, path_length, document, sizeof(document)) != file_length) return 0;
    return app_table_import_csv(table_id, document, sizeof(document), file_length);
}

/* Import one CSV table file only when the caller's equality-only model
 * revision is still current. */
int app_table_import_csv_file_if_revision(
    int table_id, const char *path_data, unsigned __int64 path_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_import_csv_file(table_id, path_data, path_length);
}

/* Import one caller-owned CSV list. Each CSV record contains exactly one
 * escaped field and becomes one list item; the temporary list is published
 * only after the complete bounded prefix is valid. */
int app_list_import_csv(int list_id, const unsigned char *input_data,
                        unsigned __int64 input_capacity,
                        unsigned __int64 input_length) {
    JadrenAppList parsed;
    JadrenAppListItem field;
    unsigned __int64 offset = 0;
    int delimiter;
    if (!app_list_valid(list_id) || (input_data == 0 && input_length > 0) ||
        input_length > input_capacity || input_length > JADREN_APP_LIST_DOCUMENT_MAX) return 0;
    app_list_clear_store(&parsed);
    while (offset < input_length) {
        if (parsed.count >= JADREN_APP_LIST_MAX_ITEMS) return 0;
        delimiter = app_table_csv_read_field(input_data, input_length, &offset, &field);
        if (delimiter != JADREN_APP_TABLE_CSV_FIELD_ROW) return 0;
        app_table_csv_copy_item(&parsed.items[parsed.count], &field);
        parsed.count += 1;
    }
    app_list_copy_store(&jadren_app_lists[list_id], &parsed);
    return 1;
}

int app_list_import_csv_if_revision(
    int list_id, const unsigned char *input_data,
    unsigned __int64 input_capacity, unsigned __int64 input_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_import_csv(list_id, input_data, input_capacity, input_length);
}

/* Import one bounded CSV list file transactionally. */
int app_list_import_csv_file(int list_id, const char *path_data,
                             unsigned __int64 path_length) {
    static unsigned char document[JADREN_APP_LIST_DOCUMENT_MAX];
    unsigned __int64 file_length;
    if (!app_list_valid(list_id) || path_data == 0 || path_length == 0) return 0;
    file_length = file_size(path_data, path_length);
    if (file_length > sizeof(document)) return 0;
    if (file_length == 0) {
        if (!file_exists(path_data, path_length)) return 0;
        app_list_clear_store(&jadren_app_lists[list_id]);
        return 1;
    }
    if (file_read(path_data, path_length, document, sizeof(document)) != file_length) return 0;
    return app_list_import_csv(list_id, document, sizeof(document), file_length);
}

int app_list_import_csv_file_if_revision(
    int list_id, const char *path_data, unsigned __int64 path_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_import_csv_file(list_id, path_data, path_length);
}

#define JADREN_APP_TABLE_SCHEMA_DOCUMENT_MAX 128
#define JADREN_APP_TABLE_FULL_SCHEMA_DOCUMENT_MAX 1024

int app_table_save_schema(int table_id, const char *path_data,
                          unsigned __int64 path_length) {
    unsigned char document[JADREN_APP_TABLE_SCHEMA_DOCUMENT_MAX];
    unsigned char number_data[4];
    unsigned __int64 offset = 0;
    unsigned __int64 number_length;
    unsigned int column_index;
    if (!app_table_valid(table_id) || path_data == 0 ||
        !app_state_append_byte(document, sizeof(document), &offset, (unsigned char)'[')) {
        return 0;
    }
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        if (column_index > 0 && !app_state_append_byte(document, sizeof(document), &offset,
                                                        (unsigned char)',')) {
            return 0;
        }
        number_length = format_int((long long)jadren_app_tables[table_id].column_types[column_index],
                                   number_data, sizeof(number_data));
        if (number_length == 0 ||
            !app_state_append_bytes(document, sizeof(document), &offset, number_data,
                                    number_length)) {
            return 0;
        }
    }
    if (!app_state_append_byte(document, sizeof(document), &offset, (unsigned char)']')) {
        return 0;
    }
    return file_write_text(path_data, path_length, (const char *)document, offset) == offset;
}

int app_table_save_schema_full(int table_id, const char *path_data,
                               unsigned __int64 path_length) {
    unsigned char document[JADREN_APP_TABLE_FULL_SCHEMA_DOCUMENT_MAX];
    unsigned char number_data[16];
    unsigned __int64 offset = 0;
    unsigned __int64 number_length;
    unsigned int column_index;
    const char *version_prefix = "{\"version\":";
    const char *columns_prefix = ",\"columns\":[";
    if (!app_table_valid(table_id) || path_data == 0 ||
        !app_state_append_bytes(document, sizeof(document), &offset,
                                 (const unsigned char *)version_prefix, 11)) return 0;
    number_length = format_int((long long)jadren_app_tables[table_id].schema_version,
                               number_data, sizeof(number_data));
    if (number_length == 0 ||
        !app_state_append_bytes(document, sizeof(document), &offset, number_data,
                                number_length) ||
        !app_state_append_bytes(document, sizeof(document), &offset,
                                (const unsigned char *)columns_prefix, 12)) return 0;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        const char *name_prefix = "{\"name\":";
        const char *kind_prefix = ",\"kind\":";
        if (column_index > 0 && !app_state_append_byte(document, sizeof(document), &offset, ',')) return 0;
        if (!app_state_append_bytes(document, sizeof(document), &offset,
                                    (const unsigned char *)name_prefix, 8) ||
            !app_state_append_json_string(document, sizeof(document), &offset,
                                           jadren_app_tables[table_id].column_names[column_index].text,
                                           jadren_app_tables[table_id].column_names[column_index].length) ||
            !app_state_append_bytes(document, sizeof(document), &offset,
                                    (const unsigned char *)kind_prefix, 8)) return 0;
        number_length = format_int((long long)jadren_app_tables[table_id].column_types[column_index],
                                   number_data, sizeof(number_data));
        if (number_length == 0 ||
            !app_state_append_bytes(document, sizeof(document), &offset, number_data, number_length) ||
            !app_state_append_byte(document, sizeof(document), &offset, '}')) return 0;
    }
    if (!app_state_append_bytes(document, sizeof(document), &offset,
                                (const unsigned char *)"]}", 2)) return 0;
    return file_write_text(path_data, path_length, (const char *)document, offset) == offset;
}

int file_replace_atomic(const char *source_data, unsigned __int64 source_length,
                        const char *target_data, unsigned __int64 target_length);

int app_table_save_atomic(int table_id,
                          const char *temporary_path_data,
                          unsigned __int64 temporary_path_length,
                          const char *target_path_data,
                          unsigned __int64 target_path_length) {
    if (!app_table_valid(table_id) || temporary_path_data == 0 || target_path_data == 0 ||
        temporary_path_length == 0 || target_path_length == 0 ||
        !app_table_save(table_id, temporary_path_data, temporary_path_length)) {
        return 0;
    }
    return file_replace_atomic(temporary_path_data, temporary_path_length,
                                target_path_data, target_path_length);
}

int app_table_save_schema_atomic(int table_id,
                                 const char *temporary_path_data,
                                 unsigned __int64 temporary_path_length,
                                 const char *target_path_data,
                                 unsigned __int64 target_path_length) {
    if (!app_table_valid(table_id) || temporary_path_data == 0 || target_path_data == 0 ||
        temporary_path_length == 0 || target_path_length == 0 ||
        !app_table_save_schema(table_id, temporary_path_data, temporary_path_length)) {
        return 0;
    }
    return file_replace_atomic(temporary_path_data, temporary_path_length,
                               target_path_data, target_path_length);
}

int app_table_save_schema_full_atomic(int table_id,
                                      const char *temporary_path_data,
                                      unsigned __int64 temporary_path_length,
                                      const char *target_path_data,
                                      unsigned __int64 target_path_length) {
    if (!app_table_valid(table_id) || temporary_path_data == 0 || target_path_data == 0 ||
        temporary_path_length == 0 || target_path_length == 0 ||
        !app_table_save_schema_full(table_id, temporary_path_data, temporary_path_length)) return 0;
    return file_replace_atomic(temporary_path_data, temporary_path_length,
                               target_path_data, target_path_length);
}

static int app_table_parse_document(const unsigned char *data,
                                    unsigned __int64 length,
                                    JadrenAppTable *table) {
    unsigned __int64 index = 0;
    unsigned __int64 string_end;
    unsigned __int64 value_length;
    unsigned int column_index;
    JadrenAppListItem *cell;
    if (data == 0 || table == 0 || length == 0 ||
        !json_read_skip_ws(data, length, &index) || data[index] != '[') {
        return 0;
    }
    index += 1;
    app_table_clear_store(table);
    for (;;) {
        while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                  data[index] == '\r' || data[index] == '\n')) {
            index += 1;
        }
        if (index >= length) {
            return 0;
        }
        if (data[index] == ']') {
            index += 1;
            while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                      data[index] == '\r' || data[index] == '\n')) {
                index += 1;
            }
            return index == length;
        }
        if (table->row_count >= JADREN_APP_TABLE_MAX_ROWS || data[index] != '[') {
            return 0;
        }
        index += 1;
        for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
            while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                      data[index] == '\r' || data[index] == '\n')) {
                index += 1;
            }
            if (index >= length || !json_read_string_end(data, length, index, &string_end)) {
                return 0;
            }
            value_length = json_read_string_length(data, index, string_end);
            if (value_length > JADREN_APP_LIST_TEXT_MAX) {
                return 0;
            }
            cell = &table->cells[table->row_count][column_index];
            cell->length = value_length;
            (void)json_read_string_copy(data, index, string_end, cell->text);
            index = string_end;
            while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                      data[index] == '\r' || data[index] == '\n')) {
                index += 1;
            }
            if (column_index + 1 < JADREN_APP_TABLE_MAX_COLUMNS) {
                if (index >= length || data[index] != ',') {
                    return 0;
                }
                index += 1;
            } else {
                if (index >= length || data[index] != ']') {
                    return 0;
                }
                index += 1;
            }
        }
        table->row_count += 1;
        while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                  data[index] == '\r' || data[index] == '\n')) {
            index += 1;
        }
        if (index >= length) {
            return 0;
        }
        if (data[index] == ',') {
            index += 1;
            continue;
        }
        if (data[index] == ']') {
            index += 1;
            while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                                      data[index] == '\r' || data[index] == '\n')) {
                index += 1;
            }
            return index == length;
        }
        return 0;
    }
}

static int app_table_parse_schema_document(const unsigned char *data,
                                           unsigned __int64 length,
                                           unsigned char *schema) {
    unsigned __int64 index = 0;
    unsigned __int64 start;
    unsigned __int64 end;
    unsigned __int64 value;
    unsigned int column_index;
    if (data == 0 || schema == 0 || length == 0 ||
        !json_read_skip_ws(data, length, &index) || data[index] != '[') {
        return 0;
    }
    index += 1;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        if (!json_read_skip_ws(data, length, &index)) return 0;
        start = index;
        while (index < length && data[index] != ',' && data[index] != ']') index += 1;
        end = index;
        while (end > start && (data[end - 1] == ' ' || data[end - 1] == '\t' ||
                               data[end - 1] == '\r' || data[end - 1] == '\n')) {
            end -= 1;
        }
        if (start == end || !json_read_uint_value(data, start, end, &value) ||
            value > JADREN_APP_TABLE_FLOAT) {
            return 0;
        }
        schema[column_index] = (unsigned char)value;
        if (column_index + 1 < JADREN_APP_TABLE_MAX_COLUMNS) {
            if (index >= length || data[index] != ',') return 0;
            index += 1;
        } else {
            if (index >= length || data[index] != ']') return 0;
            index += 1;
        }
    }
    while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                              data[index] == '\r' || data[index] == '\n')) {
        index += 1;
    }
    return index == length;
}

static int app_table_schema_full_expect_byte(const unsigned char *data,
                                             unsigned __int64 length,
                                             unsigned __int64 *index,
                                             unsigned char expected) {
    if (!json_read_skip_ws(data, length, index) || *index >= length || data[*index] != expected) return 0;
    *index += 1;
    return 1;
}

static int app_table_schema_full_expect_key(const unsigned char *data,
                                            unsigned __int64 length,
                                            unsigned __int64 *index,
                                            const char *key,
                                            unsigned __int64 key_length) {
    unsigned __int64 end;
    unsigned __int64 byte_index;
    if (!json_read_skip_ws(data, length, index) ||
        !json_read_string_end(data, length, *index, &end) ||
        json_read_string_length(data, *index, end) != key_length) return 0;
    for (byte_index = 0; byte_index < key_length; byte_index += 1)
        if (data[*index + 1 + byte_index] != (unsigned char)key[byte_index]) return 0;
    *index = end;
    return app_table_schema_full_expect_byte(data, length, index, ':');
}

static int app_table_schema_full_peek_key(const unsigned char *data,
                                          unsigned __int64 length,
                                          const unsigned __int64 *index,
                                          const char *key,
                                          unsigned __int64 key_length) {
    unsigned __int64 probe;
    unsigned __int64 end;
    unsigned __int64 byte_index;
    if (data == 0 || index == 0 || key == 0) return 0;
    probe = *index;
    if (!json_read_skip_ws(data, length, &probe) ||
        !json_read_string_end(data, length, probe, &end) ||
        json_read_string_length(data, probe, end) != key_length) return 0;
    for (byte_index = 0; byte_index < key_length; byte_index += 1)
        if (data[probe + 1 + byte_index] != (unsigned char)key[byte_index]) return 0;
    return 1;
}

static int app_table_parse_schema_full_document(const unsigned char *data,
                                                unsigned __int64 length,
                                                unsigned char *schema,
                                                JadrenAppListItem *names,
                                                int *version) {
    unsigned __int64 index = 0;
    unsigned __int64 end;
    unsigned __int64 start;
    unsigned __int64 value;
    unsigned __int64 value_length;
    unsigned int column_index;
    if (data == 0 || schema == 0 || names == 0 || version == 0 || length == 0 ||
        !app_table_schema_full_expect_byte(data, length, &index, '{')) return 0;
    *version = 0;
    if (app_table_schema_full_peek_key(data, length, &index, "version", 7)) {
        if (!app_table_schema_full_expect_key(data, length, &index, "version", 7) ||
            !json_read_skip_ws(data, length, &index)) return 0;
        start = index;
        while (index < length && data[index] != ',') index += 1;
        end = index;
        while (end > start && (data[end - 1] == ' ' || data[end - 1] == '\t' ||
                               data[end - 1] == '\r' || data[end - 1] == '\n')) end -= 1;
        if (start == end || index >= length || data[index] != ',' ||
            !json_read_uint_value(data, start, end, &value) || value > 2147483647ULL) return 0;
        *version = (int)value;
        index += 1;
    }
    if (!app_table_schema_full_expect_key(data, length, &index, "columns", 7) ||
        !app_table_schema_full_expect_byte(data, length, &index, '[')) return 0;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        if (!app_table_schema_full_expect_byte(data, length, &index, '{') ||
            !app_table_schema_full_expect_key(data, length, &index, "name", 4) ||
            !json_read_skip_ws(data, length, &index) ||
            !json_read_string_end(data, length, index, &end)) return 0;
        value_length = json_read_string_length(data, index, end);
        if (value_length > JADREN_APP_TABLE_COLUMN_NAME_MAX ||
            !app_table_column_name_valid(data + index + 1, value_length)) return 0;
        names[column_index].length = value_length;
        for (unsigned __int64 byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1)
            names[column_index].text[byte_index] = 0;
        (void)json_read_string_copy(data, index, end, names[column_index].text);
        index = end;
        if (!app_table_schema_full_expect_byte(data, length, &index, ',') ||
            !app_table_schema_full_expect_key(data, length, &index, "kind", 4) ||
            !json_read_skip_ws(data, length, &index)) return 0;
        start = index;
        while (index < length && data[index] != '}') index += 1;
        end = index;
        while (end > start && (data[end - 1] == ' ' || data[end - 1] == '\t' ||
                               data[end - 1] == '\r' || data[end - 1] == '\n')) end -= 1;
        if (start == end || !json_read_uint_value(data, start, end, &value) ||
            value > JADREN_APP_TABLE_FLOAT ||
            !app_table_schema_full_expect_byte(data, length, &index, '}')) return 0;
        schema[column_index] = (unsigned char)value;
        if (column_index + 1 < JADREN_APP_TABLE_MAX_COLUMNS) {
            if (!app_table_schema_full_expect_byte(data, length, &index, ',')) return 0;
        } else if (!app_table_schema_full_expect_byte(data, length, &index, ']')) {
            return 0;
        }
    }
    if (!app_table_schema_full_expect_byte(data, length, &index, '}')) return 0;
    while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                              data[index] == '\r' || data[index] == '\n')) index += 1;
    return index == length;
}

/* Parse the deterministic columns/rows JSON projection used by
 * app_table_export_json_exact. Column kinds remain the live table schema;
 * incoming names and bounded text-form values are validated before publish. */
static int app_table_parse_json_exact_document(const unsigned char *data,
                                               unsigned __int64 length,
                                               JadrenAppTable *table) {
    unsigned __int64 index = 0;
    unsigned __int64 end;
    unsigned __int64 value_length;
    unsigned int column_count = 0;
    unsigned int row_index;
    unsigned int column_index;
    if (data == 0 || table == 0 || length == 0 ||
        !app_table_schema_full_expect_byte(data, length, &index, (unsigned char)'{') ||
        !app_table_schema_full_expect_key(data, length, &index,
                                          "columns", 7) ||
        !app_table_schema_full_expect_byte(data, length, &index, (unsigned char)'[')) {
        return 0;
    }
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1)
        app_list_clear_item(&table->column_names[column_index]);
    for (;;) {
        if (!json_read_skip_ws(data, length, &index) || index >= length) return 0;
        if (data[index] == (unsigned char)']') {
            index += 1;
            break;
        }
        if (column_count >= JADREN_APP_TABLE_MAX_COLUMNS ||
            !json_read_string_end(data, length, index, &end)) return 0;
        value_length = json_read_string_length(data, index, end);
        if (value_length > JADREN_APP_TABLE_COLUMN_NAME_MAX ||
            !app_table_column_name_valid(data + index + 1, value_length)) return 0;
        table->column_names[column_count].length = value_length;
        (void)json_read_string_copy(data, index, end,
                                    table->column_names[column_count].text);
        column_count += 1;
        index = end;
        if (!json_read_skip_ws(data, length, &index) || index >= length) return 0;
        if (data[index] == (unsigned char)',') {
            index += 1;
            continue;
        }
        if (data[index] == (unsigned char)']') {
            index += 1;
            break;
        }
        return 0;
    }
    if (!app_table_schema_full_expect_byte(data, length, &index, (unsigned char)',') ||
        !app_table_schema_full_expect_key(data, length, &index, "rows", 4) ||
        !app_table_schema_full_expect_byte(data, length, &index, (unsigned char)'[')) {
        return 0;
    }
    app_table_clear_store(table);
    for (;;) {
        if (!json_read_skip_ws(data, length, &index) || index >= length) return 0;
        if (data[index] == (unsigned char)']') {
            index += 1;
            break;
        }
        if (column_count == 0 || table->row_count >= JADREN_APP_TABLE_MAX_ROWS ||
            data[index] != (unsigned char)'[') return 0;
        index += 1;
        row_index = (unsigned int)table->row_count;
        for (column_index = 0; column_index < column_count; column_index += 1) {
            if (!json_read_skip_ws(data, length, &index) ||
                !json_read_string_end(data, length, index, &end)) return 0;
            value_length = json_read_string_length(data, index, end);
            if (value_length > JADREN_APP_LIST_TEXT_MAX) return 0;
            table->cells[row_index][column_index].length = value_length;
            (void)json_read_string_copy(data, index, end,
                                        table->cells[row_index][column_index].text);
            index = end;
            if (!json_read_skip_ws(data, length, &index) || index >= length) return 0;
            if (column_index + 1 < column_count) {
                if (data[index] != (unsigned char)',') return 0;
                index += 1;
            } else {
                if (data[index] != (unsigned char)']') return 0;
                index += 1;
            }
        }
        table->row_count += 1;
        if (!json_read_skip_ws(data, length, &index) || index >= length) return 0;
        if (data[index] == (unsigned char)',') {
            index += 1;
            continue;
        }
        if (data[index] == (unsigned char)']') {
            index += 1;
            break;
        }
        return 0;
    }
    if (!app_table_schema_full_expect_byte(data, length, &index, (unsigned char)'}')) return 0;
    while (index < length && (data[index] == ' ' || data[index] == '\t' ||
                              data[index] == '\r' || data[index] == '\n')) index += 1;
    return index == length;
}

/* Import one caller-owned table JSON projection transactionally. */
int app_table_import_json_exact(int table_id, const unsigned char *input_data,
                                unsigned __int64 input_capacity,
                                unsigned __int64 input_length) {
    JadrenAppTable parsed;
    if (!app_table_valid(table_id) || input_data == 0 || input_length == 0 ||
        input_length > input_capacity || input_length > JADREN_APP_TABLE_DOCUMENT_MAX) {
        return 0;
    }
    app_table_copy_schema(&parsed, &jadren_app_tables[table_id]);
    if (!app_table_parse_json_exact_document(input_data, input_length, &parsed) ||
        !app_table_store_valid(&parsed)) return 0;
    app_table_copy_store(&jadren_app_tables[table_id], &parsed);
    return 1;
}

/* Import one JSON table projection only when the caller's equality-only model
 * revision is still current. The typed import remains transactional. */
int app_table_import_json_exact_if_revision(
    int table_id, const unsigned char *input_data,
    unsigned __int64 input_capacity, unsigned __int64 input_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_import_json_exact(
        table_id, input_data, input_capacity, input_length);
}

/* Import one deterministic columns/rows JSON file transactionally while
 * retaining the live table's typed column schema. */
int app_table_import_json_file(int table_id, const char *path_data,
                               unsigned __int64 path_length) {
    static unsigned char document[JADREN_APP_TABLE_DOCUMENT_MAX];
    JadrenAppTable parsed;
    unsigned __int64 document_length;
    unsigned __int64 file_length;
    if (!app_table_valid(table_id) || path_data == 0 || path_length == 0) return 0;
    file_length = file_size(path_data, path_length);
    if (file_length == 0 || file_length > sizeof(document)) return 0;
    document_length = file_read(path_data, path_length, document, sizeof(document));
    if (document_length != file_length) return 0;
    app_table_copy_schema(&parsed, &jadren_app_tables[table_id]);
    if (!app_table_parse_json_exact_document(document, document_length, &parsed) ||
        !app_table_store_valid(&parsed)) return 0;
    app_table_copy_store(&jadren_app_tables[table_id], &parsed);
    return 1;
}

/* Import one JSON table file only when the caller's equality-only model
 * revision is still current. The typed file parser remains transactional. */
int app_table_import_json_file_if_revision(
    int table_id, const char *path_data, unsigned __int64 path_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_import_json_file(table_id, path_data, path_length);
}

int app_table_load(int table_id, const char *path_data, unsigned __int64 path_length) {
    unsigned char document[JADREN_APP_TABLE_DOCUMENT_MAX];
    JadrenAppTable parsed;
    unsigned __int64 document_length;
    unsigned int row_index;
    unsigned int column_index;
    unsigned __int64 byte_index;
    if (!app_table_valid(table_id) || path_data == 0 ||
        file_size(path_data, path_length) > JADREN_APP_TABLE_DOCUMENT_MAX) {
        return 0;
    }
    document_length = file_read(path_data, path_length, document, sizeof(document));
    app_table_copy_schema(&parsed, &jadren_app_tables[table_id]);
    app_table_clear_store(&parsed);
    if (!app_table_parse_document(document, document_length, &parsed)) {
        return 0;
    }
    if (!app_table_store_valid(&parsed)) return 0;
    jadren_app_tables[table_id].row_count = parsed.row_count;
    for (row_index = 0; row_index < JADREN_APP_TABLE_MAX_ROWS; row_index += 1) {
        for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
            JadrenAppListItem *destination = &jadren_app_tables[table_id].cells[row_index][column_index];
            JadrenAppListItem *source = &parsed.cells[row_index][column_index];
            destination->length = source->length;
            for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1) {
                destination->text[byte_index] = source->text[byte_index];
            }
        }
    }
    return 1;
}

int app_table_load_schema(int table_id, const char *path_data,
                          unsigned __int64 path_length) {
    unsigned char document[JADREN_APP_TABLE_SCHEMA_DOCUMENT_MAX];
    unsigned char parsed_schema[JADREN_APP_TABLE_MAX_COLUMNS];
    unsigned char previous_schema[JADREN_APP_TABLE_MAX_COLUMNS];
    unsigned __int64 document_length;
    unsigned int column_index;
    if (!app_table_valid(table_id) || path_data == 0 ||
        file_size(path_data, path_length) > JADREN_APP_TABLE_SCHEMA_DOCUMENT_MAX) {
        return 0;
    }
    document_length = file_read(path_data, path_length, document, sizeof(document));
    if (!app_table_parse_schema_document(document, document_length, parsed_schema)) return 0;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        previous_schema[column_index] = jadren_app_tables[table_id].column_types[column_index];
        jadren_app_tables[table_id].column_types[column_index] = parsed_schema[column_index];
    }
    if (!app_table_store_valid(&jadren_app_tables[table_id])) {
        for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
            jadren_app_tables[table_id].column_types[column_index] = previous_schema[column_index];
        }
        return 0;
    }
    return 1;
}

static int app_table_load_schema_full_internal(int table_id, const char *path_data,
                                               unsigned __int64 path_length,
                                               int expected_version,
                                               int enforce_version) {
    unsigned char document[JADREN_APP_TABLE_FULL_SCHEMA_DOCUMENT_MAX];
    unsigned char parsed_schema[JADREN_APP_TABLE_MAX_COLUMNS];
    unsigned char previous_schema[JADREN_APP_TABLE_MAX_COLUMNS];
    JadrenAppListItem parsed_names[JADREN_APP_TABLE_MAX_COLUMNS];
    JadrenAppListItem previous_names[JADREN_APP_TABLE_MAX_COLUMNS];
    unsigned __int64 document_length;
    unsigned int column_index;
    unsigned __int64 byte_index;
    int parsed_version;
    int previous_version;
    if (!app_table_valid(table_id) || path_data == 0 ||
        file_size(path_data, path_length) > JADREN_APP_TABLE_FULL_SCHEMA_DOCUMENT_MAX) return 0;
    document_length = file_read(path_data, path_length, document, sizeof(document));
    if (!app_table_parse_schema_full_document(document, document_length,
                                              parsed_schema, parsed_names,
                                              &parsed_version) ||
        (enforce_version && parsed_version != expected_version)) return 0;
    previous_version = jadren_app_tables[table_id].schema_version;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        previous_schema[column_index] = jadren_app_tables[table_id].column_types[column_index];
        previous_names[column_index].length = jadren_app_tables[table_id].column_names[column_index].length;
        for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1)
            previous_names[column_index].text[byte_index] = jadren_app_tables[table_id].column_names[column_index].text[byte_index];
        jadren_app_tables[table_id].column_types[column_index] = parsed_schema[column_index];
        jadren_app_tables[table_id].column_names[column_index].length = parsed_names[column_index].length;
        for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1)
            jadren_app_tables[table_id].column_names[column_index].text[byte_index] = parsed_names[column_index].text[byte_index];
    }
    jadren_app_tables[table_id].schema_version = parsed_version;
    if (!app_table_store_valid(&jadren_app_tables[table_id])) {
        for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
            jadren_app_tables[table_id].column_types[column_index] = previous_schema[column_index];
            jadren_app_tables[table_id].column_names[column_index].length = previous_names[column_index].length;
            for (byte_index = 0; byte_index < JADREN_APP_LIST_TEXT_MAX; byte_index += 1)
                jadren_app_tables[table_id].column_names[column_index].text[byte_index] = previous_names[column_index].text[byte_index];
        }
        jadren_app_tables[table_id].schema_version = previous_version;
        return 0;
    }
    return 1;
}

int app_table_load_schema_full(int table_id, const char *path_data,
                               unsigned __int64 path_length) {
    return app_table_load_schema_full_internal(table_id, path_data, path_length, 0, 0);
}

int app_table_load_schema_full_if_version(int table_id, const char *path_data,
                                          unsigned __int64 path_length,
                                          int expected_version) {
    return app_table_load_schema_full_internal(table_id, path_data, path_length,
                                               expected_version, 1);
}

#define JADREN_APP_DATA_DOCUMENT_MAX 4194304

static int app_data_build_state_document(unsigned char *document,
                                         unsigned __int64 document_length,
                                         unsigned __int64 *written) {
    unsigned __int64 offset = 0;
    unsigned char number_data[32];
    unsigned int entry_index;
    int first = 1;
    if (document == 0 || written == 0 ||
        !app_state_append_byte(document, document_length, &offset, '{')) return 0;
    for (entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1) {
        JadrenAppStateEntry *entry = &jadren_app_state[entry_index];
        unsigned __int64 number_length;
        if (!entry->used || entry->kind == 0) continue;
        if (!first && !app_state_append_byte(document, document_length, &offset, ',')) return 0;
        first = 0;
        if (!app_state_append_json_string(document, document_length, &offset,
                                          entry->key, entry->key_length) ||
            !app_state_append_byte(document, document_length, &offset, ':')) return 0;
        if (entry->kind == JADREN_APP_STATE_TEXT) {
            if (!app_state_append_json_string(document, document_length, &offset,
                                              entry->text, entry->text_length)) return 0;
        } else if (entry->kind == JADREN_APP_STATE_BOOL) {
            const unsigned char *value = entry->bool_value ?
                (const unsigned char *)"true" : (const unsigned char *)"false";
            if (!app_state_append_bytes(document, document_length, &offset,
                                        value, entry->bool_value ? 4 : 5)) return 0;
        } else if (entry->kind == JADREN_APP_STATE_INT) {
            number_length = format_int(entry->int_value, number_data, sizeof(number_data));
            if (number_length == 0 || !app_state_append_bytes(document, document_length,
                                                                &offset, number_data, number_length)) return 0;
        } else if (entry->kind == JADREN_APP_STATE_UINT) {
            number_length = format_uint(entry->uint_value, number_data, sizeof(number_data));
            if (number_length == 0 || !app_state_append_bytes(document, document_length,
                                                                &offset, number_data, number_length)) return 0;
        } else return 0;
    }
    if (!app_state_append_byte(document, document_length, &offset, '}')) return 0;
    *written = offset;
    return 1;
}

static int app_data_build_list_document(int list_id, unsigned char *document,
                                        unsigned __int64 document_length,
                                        unsigned __int64 *written) {
    unsigned __int64 offset = 0;
    unsigned __int64 index;
    if (!app_list_valid(list_id) || document == 0 || written == 0 ||
        !app_state_append_byte(document, document_length, &offset, '[')) return 0;
    for (index = 0; index < jadren_app_lists[list_id].count; index += 1) {
        JadrenAppListItem *item = &jadren_app_lists[list_id].items[index];
        if (index > 0 && !app_state_append_byte(document, document_length, &offset, ',')) return 0;
        if (!app_state_append_json_string(document, document_length, &offset,
                                          item->text, item->length)) return 0;
    }
    if (!app_state_append_byte(document, document_length, &offset, ']')) return 0;
    *written = offset;
    return 1;
}

static int app_data_build_table_rows_document(int table_id, unsigned char *document,
                                               unsigned __int64 document_length,
                                               unsigned __int64 *written) {
    unsigned __int64 offset = 0;
    unsigned int row_index;
    unsigned int column_index;
    if (!app_table_valid(table_id) || document == 0 || written == 0 ||
        !app_state_append_byte(document, document_length, &offset, '[')) return 0;
    for (row_index = 0; row_index < jadren_app_tables[table_id].row_count; row_index += 1) {
        if (row_index > 0 && !app_state_append_byte(document, document_length, &offset, ',')) return 0;
        if (!app_state_append_byte(document, document_length, &offset, '[')) return 0;
        for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
            JadrenAppListItem *cell = &jadren_app_tables[table_id].cells[row_index][column_index];
            if (column_index > 0 && !app_state_append_byte(document, document_length, &offset, ',')) return 0;
            if (!app_state_append_json_string(document, document_length, &offset,
                                              cell->text, cell->length)) return 0;
        }
        if (!app_state_append_byte(document, document_length, &offset, ']')) return 0;
    }
    if (!app_state_append_byte(document, document_length, &offset, ']')) return 0;
    *written = offset;
    return 1;
}

static int app_data_build_table_schema_document(int table_id, unsigned char *document,
                                                unsigned __int64 document_length,
                                                unsigned __int64 *written) {
    unsigned char number_data[16];
    unsigned __int64 offset = 0;
    unsigned __int64 number_length;
    unsigned int column_index;
    if (!app_table_valid(table_id) || document == 0 || written == 0 ||
        !app_state_append_bytes(document, document_length, &offset,
                                 (const unsigned char *)"{\"version\":", 11)) return 0;
    number_length = format_int((long long)jadren_app_tables[table_id].schema_version,
                               number_data, sizeof(number_data));
    if (number_length == 0 || !app_state_append_bytes(document, document_length, &offset,
                                                        number_data, number_length) ||
        !app_state_append_bytes(document, document_length, &offset,
                                (const unsigned char *)",\"columns\":[", 12)) return 0;
    for (column_index = 0; column_index < JADREN_APP_TABLE_MAX_COLUMNS; column_index += 1) {
        JadrenAppListItem *name = &jadren_app_tables[table_id].column_names[column_index];
        if (column_index > 0 && !app_state_append_byte(document, document_length, &offset, ',')) return 0;
        if (!app_state_append_bytes(document, document_length, &offset,
                                    (const unsigned char *)"{\"name\":", 8) ||
            !app_state_append_json_string(document, document_length, &offset,
                                           name->text, name->length) ||
            !app_state_append_bytes(document, document_length, &offset,
                                    (const unsigned char *)",\"kind\":", 8)) return 0;
        number_length = format_int((long long)jadren_app_tables[table_id].column_types[column_index],
                                   number_data, sizeof(number_data));
        if (number_length == 0 || !app_state_append_bytes(document, document_length, &offset,
                                                            number_data, number_length) ||
            !app_state_append_byte(document, document_length, &offset, '}')) return 0;
    }
    if (!app_state_append_bytes(document, document_length, &offset,
                                (const unsigned char *)"]}", 2)) return 0;
    *written = offset;
    return 1;
}

static int app_data_append_uint(unsigned char *document, unsigned __int64 document_length,
                                unsigned __int64 *offset, unsigned __int64 value) {
    unsigned char number_data[32];
    unsigned __int64 number_length = format_uint(value, number_data, sizeof(number_data));
    return number_length > 0 && app_state_append_bytes(document, document_length, offset,
                                                        number_data, number_length);
}

static int app_data_append_segment_header(unsigned char *document,
                                          unsigned __int64 document_length,
                                          unsigned __int64 *offset, const char *label,
                                          unsigned __int64 label_length,
                                          unsigned __int64 payload_length) {
    return app_state_append_bytes(document, document_length, offset,
                                  (const unsigned char *)label, label_length) &&
           app_state_append_byte(document, document_length, offset, ':') &&
           app_data_append_uint(document, document_length, offset, payload_length) &&
           app_state_append_byte(document, document_length, offset, '\n');
}

static int app_data_append_table_header(unsigned char *document,
                                        unsigned __int64 document_length,
                                        unsigned __int64 *offset, int table_id,
                                        unsigned __int64 schema_length,
                                        unsigned __int64 rows_length) {
    unsigned char label[16];
    unsigned __int64 label_length = format_int(table_id, label, sizeof(label));
    return label_length > 0 && app_state_append_bytes(document, document_length, offset,
                                                       (const unsigned char *)"TABLE", 5) &&
           app_state_append_bytes(document, document_length, offset, label, label_length) &&
           app_state_append_byte(document, document_length, offset, ':') &&
           app_data_append_uint(document, document_length, offset, schema_length) &&
           app_state_append_byte(document, document_length, offset, ':') &&
           app_data_append_uint(document, document_length, offset, rows_length) &&
           app_state_append_byte(document, document_length, offset, '\n');
}

int app_data_save(const char *path_data, unsigned __int64 path_length) {
    static unsigned char document[JADREN_APP_DATA_DOCUMENT_MAX];
    static unsigned char state_document[JADREN_APP_STATE_DOCUMENT_MAX];
    static unsigned char list_document[JADREN_APP_LIST_DOCUMENT_MAX];
    static unsigned char table_rows_document[JADREN_APP_TABLE_DOCUMENT_MAX];
    static unsigned char table_schema_document[JADREN_APP_TABLE_FULL_SCHEMA_DOCUMENT_MAX];
    unsigned __int64 offset = 0;
    unsigned __int64 written;
    int list_id;
    int table_id;
    if (path_data == 0 || path_length == 0 ||
        !app_state_append_bytes(document, sizeof(document), &offset,
                                 (const unsigned char *)"JADREN-APP-DATA-0.1\n", 20) ||
        !app_data_build_state_document(state_document, sizeof(state_document), &written) ||
        !app_data_append_segment_header(document, sizeof(document), &offset,
                                        "STATE", 5, written) ||
        !app_state_append_bytes(document, sizeof(document), &offset, state_document, written)) return 0;
    for (list_id = 0; list_id < JADREN_APP_LIST_MAX_LISTS; list_id += 1) {
        if (!app_data_build_list_document(list_id, list_document, sizeof(list_document), &written) ||
            !app_data_append_segment_header(document, sizeof(document), &offset,
                                            list_id == 0 ? "LIST0" : list_id == 1 ? "LIST1" :
                                            list_id == 2 ? "LIST2" : "LIST3", 5, written) ||
            !app_state_append_bytes(document, sizeof(document), &offset, list_document, written)) return 0;
    }
    for (table_id = 0; table_id < JADREN_APP_TABLE_MAX_TABLES; table_id += 1) {
        unsigned __int64 schema_length;
        unsigned __int64 rows_length;
        if (!app_data_build_table_schema_document(table_id, table_schema_document,
                                                  sizeof(table_schema_document), &schema_length) ||
            !app_data_build_table_rows_document(table_id, table_rows_document,
                                                sizeof(table_rows_document), &rows_length) ||
            !app_data_append_table_header(document, sizeof(document), &offset, table_id,
                                          schema_length, rows_length) ||
            !app_state_append_bytes(document, sizeof(document), &offset,
                                    table_schema_document, schema_length) ||
            !app_state_append_bytes(document, sizeof(document), &offset,
                                    table_rows_document, rows_length)) return 0;
    }
    return file_write_text(path_data, path_length, (const char *)document, offset) == offset;
}

/* Export the complete bounded application model into caller-owned memory.
 * The document is assembled before copying, so short output buffers never
 * receive a partial model or a length update. */
int app_data_write_exact(unsigned char *output_data,
                         unsigned __int64 output_length,
                         unsigned __int64 *written_data,
                         unsigned __int64 written_capacity) {
    static unsigned char document[JADREN_APP_DATA_DOCUMENT_MAX];
    static unsigned char state_document[JADREN_APP_STATE_DOCUMENT_MAX];
    static unsigned char list_document[JADREN_APP_LIST_DOCUMENT_MAX];
    static unsigned char table_rows_document[JADREN_APP_TABLE_DOCUMENT_MAX];
    static unsigned char table_schema_document[JADREN_APP_TABLE_FULL_SCHEMA_DOCUMENT_MAX];
    unsigned __int64 offset = 0;
    unsigned __int64 written;
    unsigned __int64 byte_index;
    int list_id;
    int table_id;
    if (output_data == 0 || output_length == 0 || written_data == 0 ||
        written_capacity == 0 ||
        !app_state_append_bytes(document, sizeof(document), &offset,
                                 (const unsigned char *)"JADREN-APP-DATA-0.1\n", 20) ||
        !app_data_build_state_document(state_document, sizeof(state_document), &written) ||
        !app_data_append_segment_header(document, sizeof(document), &offset,
                                        "STATE", 5, written) ||
        !app_state_append_bytes(document, sizeof(document), &offset, state_document, written)) return 0;
    for (list_id = 0; list_id < JADREN_APP_LIST_MAX_LISTS; list_id += 1) {
        if (!app_data_build_list_document(list_id, list_document, sizeof(list_document), &written) ||
            !app_data_append_segment_header(document, sizeof(document), &offset,
                                            list_id == 0 ? "LIST0" : list_id == 1 ? "LIST1" :
                                            list_id == 2 ? "LIST2" : "LIST3", 5, written) ||
            !app_state_append_bytes(document, sizeof(document), &offset, list_document, written)) return 0;
    }
    for (table_id = 0; table_id < JADREN_APP_TABLE_MAX_TABLES; table_id += 1) {
        unsigned __int64 schema_length;
        unsigned __int64 rows_length;
        if (!app_data_build_table_schema_document(table_id, table_schema_document,
                                                  sizeof(table_schema_document), &schema_length) ||
            !app_data_build_table_rows_document(table_id, table_rows_document,
                                                sizeof(table_rows_document), &rows_length) ||
            !app_data_append_table_header(document, sizeof(document), &offset, table_id,
                                          schema_length, rows_length) ||
            !app_state_append_bytes(document, sizeof(document), &offset,
                                    table_schema_document, schema_length) ||
            !app_state_append_bytes(document, sizeof(document), &offset,
                                    table_rows_document, rows_length)) return 0;
    }
    if (offset > output_length) return 0;
    for (byte_index = 0; byte_index < offset; byte_index += 1)
        output_data[byte_index] = document[byte_index];
    written_data[0] = offset;
    return 1;
}

/* Return the exact serialized size without exposing or mutating caller
 * storage. The fixed scratch buffer keeps the query bounded and mirrors the
 * same serializer used by app_data_write_exact. */
unsigned __int64 app_data_snapshot_length(void) {
    static unsigned char document[JADREN_APP_DATA_DOCUMENT_MAX];
    unsigned __int64 length = 0;
    if (!app_data_write_exact(document, sizeof(document), &length, 1)) return 0;
    return length;
}

/* Return the exact serialized size only when the model revision is stable. */
unsigned __int64 app_data_snapshot_length_if_revision(unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    unsigned __int64 length = app_data_snapshot_length();
    if (length == 0) return 0;
    if (app_data_revision() != expected_revision) return 0;
    return length;
}

/* Export only when the caller's equality-only snapshot is still current.
 * The stale path returns before serialization, so caller-owned output and
 * length remain unchanged. */
int app_data_write_exact_if_revision(unsigned char *output_data,
                                     unsigned __int64 output_length,
                                     unsigned __int64 *written_data,
                                     unsigned __int64 written_capacity,
                                     unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_data_write_exact(output_data, output_length, written_data,
                                written_capacity);
}

int app_data_save_atomic(const char *temporary_path_data,
                         unsigned __int64 temporary_path_length,
                         const char *target_path_data,
                         unsigned __int64 target_path_length) {
    if (temporary_path_data == 0 || target_path_data == 0 || temporary_path_length == 0 ||
        target_path_length == 0 || !app_data_save(temporary_path_data, temporary_path_length)) return 0;
    return file_replace_atomic(temporary_path_data, temporary_path_length,
                               target_path_data, target_path_length);
}

int app_data_save_atomic_if_revision(const char *temporary_path_data,
                                     unsigned __int64 temporary_path_length,
                                     const char *target_path_data,
                                     unsigned __int64 target_path_length,
                                     unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_data_save_atomic(temporary_path_data, temporary_path_length,
                                target_path_data, target_path_length);
}

int app_data_tx_save_atomic(const char *temporary_path_data,
                            unsigned __int64 temporary_path_length,
                            const char *target_path_data,
                            unsigned __int64 target_path_length) {
    if (!jadren_app_data_transaction_active) return 0;
    if (!app_data_save_atomic(temporary_path_data, temporary_path_length,
                              target_path_data, target_path_length)) {
        (void)app_data_tx_rollback();
        return 0;
    }
    return app_data_tx_commit();
}

unsigned __int64 file_lock_path_retry(const unsigned char *path_data,
                                      unsigned __int64 path_capacity,
                                      unsigned __int64 path_length,
                                      unsigned __int64 max_attempts,
                                      unsigned __int64 retry_delay_ms);

/* Durable model commit: serialize the active transaction into a caller-owned
 * cross-process lock, serialize into the caller-owned temporary file, flush its
 * contents, atomically replace the target, flush the promoted file, then commit
 * the in-memory snapshot. The lock path must be separate from target_path.
 * A failure after replacement can leave the new file on disk; callers should
 * reload it before retrying if the function returns false. */
int app_data_tx_commit_durable(const char *temporary_path_data,
                               unsigned __int64 temporary_path_length,
                               const char *target_path_data,
                               unsigned __int64 target_path_length,
                               const char *lock_path_data,
                               unsigned __int64 lock_path_length) {
    unsigned __int64 lock_token;
    int committed;
    if (!jadren_app_data_transaction_active || temporary_path_data == 0 ||
        target_path_data == 0 || lock_path_data == 0 || temporary_path_length == 0 ||
        target_path_length == 0 || lock_path_length == 0) return 0;
    lock_token = file_lock(lock_path_data, lock_path_length);
    if (lock_token == 0) {
        (void)app_data_tx_rollback();
        return 0;
    }
    if (!app_data_save(temporary_path_data, temporary_path_length) ||
        !file_flush(temporary_path_data, temporary_path_length)) {
        (void)file_unlock(lock_token);
        (void)file_delete(temporary_path_data, temporary_path_length);
        (void)app_data_tx_rollback();
        return 0;
    }
    if (!file_replace_atomic(temporary_path_data, temporary_path_length,
                             target_path_data, target_path_length)) {
        (void)file_unlock(lock_token);
        (void)file_delete(temporary_path_data, temporary_path_length);
        (void)app_data_tx_rollback();
        return 0;
    }
    if (!file_flush(target_path_data, target_path_length)) {
        (void)file_unlock(lock_token);
        (void)app_data_tx_commit();
        return 0;
    }
    committed = app_data_tx_commit();
    if (!file_unlock(lock_token)) return 0;
    return committed;
}

static int app_data_path_equal(const char *left_data, unsigned __int64 left_length,
                               const char *right_data, unsigned __int64 right_length) {
    unsigned __int64 index;
    if (left_data == 0 || right_data == 0 || left_length != right_length) return 0;
    for (index = 0; index < left_length; index += 1)
        if (left_data[index] != right_data[index]) return 0;
    return 1;
}

/* Durable model commit with an explicit directory metadata flush. The lock
 * remains caller-owned and the new directory path must already exist. */
int app_data_tx_commit_durable_directory(
    const char *temporary_path_data,
    unsigned __int64 temporary_path_length,
    const char *target_path_data,
    unsigned __int64 target_path_length,
    const char *directory_path_data,
    unsigned __int64 directory_path_length,
    const char *lock_path_data,
    unsigned __int64 lock_path_length) {
    unsigned __int64 lock_token;
    int committed;
    if (!jadren_app_data_transaction_active || temporary_path_data == 0 ||
        target_path_data == 0 || directory_path_data == 0 || lock_path_data == 0 ||
        temporary_path_length == 0 || target_path_length == 0 ||
        directory_path_length == 0 || lock_path_length == 0 ||
        !directory_exists(directory_path_data, directory_path_length) ||
        app_data_path_equal(temporary_path_data, temporary_path_length,
                            target_path_data, target_path_length) ||
        app_data_path_equal(temporary_path_data, temporary_path_length,
                            lock_path_data, lock_path_length) ||
        app_data_path_equal(target_path_data, target_path_length,
                            lock_path_data, lock_path_length)) return 0;
    lock_token = file_lock(lock_path_data, lock_path_length);
    if (lock_token == 0) {
        (void)app_data_tx_rollback();
        return 0;
    }
    if (!app_data_save(temporary_path_data, temporary_path_length) ||
        !file_flush(temporary_path_data, temporary_path_length) ||
        !file_replace_atomic(temporary_path_data, temporary_path_length,
                             target_path_data, target_path_length)) {
        (void)file_unlock(lock_token);
        (void)file_delete(temporary_path_data, temporary_path_length);
        (void)app_data_tx_rollback();
        return 0;
    }
    if (!file_flush(target_path_data, target_path_length) ||
        !directory_flush(directory_path_data, directory_path_length)) {
        (void)file_unlock(lock_token);
        (void)app_data_tx_commit();
        return 0;
    }
    committed = app_data_tx_commit();
    if (!file_unlock(lock_token)) return 0;
    return committed;
}

int app_data_tx_commit_durable_directory_if_revision(
    const char *temporary_path_data,
    unsigned __int64 temporary_path_length,
    const char *target_path_data,
    unsigned __int64 target_path_length,
    const char *directory_path_data,
    unsigned __int64 directory_path_length,
    const char *lock_path_data,
    unsigned __int64 lock_path_length,
    unsigned __int64 expected_revision) {
    if (!jadren_app_data_transaction_active ||
        app_data_revision() != expected_revision) return 0;
    return app_data_tx_commit_durable_directory(
        temporary_path_data, temporary_path_length, target_path_data,
        target_path_length, directory_path_data, directory_path_length,
        lock_path_data, lock_path_length);
}

/* Durable model commit with a bounded caller-selected retry for the sidecar
 * lock. The retry delay blocks only the caller thread; it is not a worker,
 * scheduler, callback, or real-time synchronization primitive. A retry
 * exhaustion uses the same rollback policy as the nonblocking durable commit. */
int app_data_tx_commit_durable_retry(const char *temporary_path_data,
                                     unsigned __int64 temporary_path_length,
                                     const char *target_path_data,
                                     unsigned __int64 target_path_length,
                                     const char *lock_path_data,
                                     unsigned __int64 lock_path_length,
                                     unsigned __int64 max_attempts,
                                     unsigned __int64 retry_delay_ms) {
    unsigned __int64 lock_token;
    int committed;
    if (!jadren_app_data_transaction_active || temporary_path_data == 0 ||
        target_path_data == 0 || lock_path_data == 0 || temporary_path_length == 0 ||
        target_path_length == 0 || lock_path_length == 0) return 0;
    lock_token = file_lock_path_retry((const unsigned char *)lock_path_data,
                                      lock_path_length, lock_path_length,
                                      max_attempts, retry_delay_ms);
    if (lock_token == 0) {
        (void)app_data_tx_rollback();
        return 0;
    }
    if (!app_data_save(temporary_path_data, temporary_path_length) ||
        !file_flush(temporary_path_data, temporary_path_length)) {
        (void)file_unlock(lock_token);
        (void)file_delete(temporary_path_data, temporary_path_length);
        (void)app_data_tx_rollback();
        return 0;
    }
    if (!file_replace_atomic(temporary_path_data, temporary_path_length,
                             target_path_data, target_path_length)) {
        (void)file_unlock(lock_token);
        (void)file_delete(temporary_path_data, temporary_path_length);
        (void)app_data_tx_rollback();
        return 0;
    }
    if (!file_flush(target_path_data, target_path_length)) {
        (void)file_unlock(lock_token);
        (void)app_data_tx_commit();
        return 0;
    }
    committed = app_data_tx_commit();
    if (!file_unlock(lock_token)) return 0;
    return committed;
}

/* Retry the durable commit only when the caller's equality-only revision is
 * still current. A stale call returns before lock acquisition, sleep, file
 * mutation, or transaction rollback so the caller retains the active snapshot. */
int app_data_tx_commit_durable_retry_if_revision(const char *temporary_path_data,
                                                 unsigned __int64 temporary_path_length,
                                                 const char *target_path_data,
                                                 unsigned __int64 target_path_length,
                                                 const char *lock_path_data,
                                                 unsigned __int64 lock_path_length,
                                                 unsigned __int64 expected_revision,
                                                 unsigned __int64 max_attempts,
                                                 unsigned __int64 retry_delay_ms) {
    if (!jadren_app_data_transaction_active || app_data_revision() != expected_revision) return 0;
    return app_data_tx_commit_durable_retry(temporary_path_data, temporary_path_length,
                                            target_path_data, target_path_length,
                                            lock_path_data, lock_path_length,
                                            max_attempts, retry_delay_ms);
}

/* Commit the active model transaction durably only when the caller's
 * equality-only revision is still current. A stale call does not acquire the
 * lock or mutate the transaction; callers may explicitly roll it back or
 * retry with a fresh revision. */
int app_data_tx_commit_durable_if_revision(const char *temporary_path_data,
                                           unsigned __int64 temporary_path_length,
                                           const char *target_path_data,
                                           unsigned __int64 target_path_length,
                                           const char *lock_path_data,
                                           unsigned __int64 lock_path_length,
                                           unsigned __int64 expected_revision) {
    if (!jadren_app_data_transaction_active || app_data_revision() != expected_revision) return 0;
    return app_data_tx_commit_durable(temporary_path_data, temporary_path_length,
                                      target_path_data, target_path_length,
                                      lock_path_data, lock_path_length);
}

static int app_data_expect_bytes(const unsigned char *data, unsigned __int64 length,
                                 unsigned __int64 *index, const char *expected,
                                 unsigned __int64 expected_length) {
    if (data == 0 || index == 0 || *index > length || expected_length > length - *index) return 0;
    for (unsigned __int64 byte_index = 0; byte_index < expected_length; byte_index += 1)
        if (data[*index + byte_index] != (unsigned char)expected[byte_index]) return 0;
    *index += expected_length;
    return 1;
}

static int app_data_read_uint_until(const unsigned char *data, unsigned __int64 length,
                                    unsigned __int64 *index, unsigned char terminator,
                                    unsigned __int64 *value) {
    unsigned __int64 parsed = 0;
    unsigned __int64 start;
    if (data == 0 || index == 0 || value == 0 || *index >= length) return 0;
    start = *index;
    while (*index < length && data[*index] != terminator) {
        unsigned char digit = data[*index];
        if (digit < '0' || digit > '9' || parsed > (((unsigned __int64)-1) - (digit - '0')) / 10) return 0;
        parsed = parsed * 10 + (digit - '0');
        *index += 1;
    }
    if (*index == start || *index >= length || data[*index] != terminator) return 0;
    *index += 1;
    *value = parsed;
    return 1;
}

static int app_data_read_segment(const unsigned char *data, unsigned __int64 length,
                                 unsigned __int64 *index, const char *label,
                                 unsigned __int64 label_length,
                                 const unsigned char **payload,
                                 unsigned __int64 *payload_length) {
    unsigned __int64 available;
    if (!app_data_expect_bytes(data, length, index, label, label_length) ||
        !app_data_expect_bytes(data, length, index, ":", 1) ||
        !app_data_read_uint_until(data, length, index, '\n', payload_length) ||
        *payload_length > length - *index) return 0;
    available = *payload_length;
    *payload = data + *index;
    *index += available;
    return 1;
}

static int app_data_read_table_segment(const unsigned char *data, unsigned __int64 length,
                                       unsigned __int64 *index, int table_id,
                                       const unsigned char **schema,
                                       unsigned __int64 *schema_length,
                                       const unsigned char **rows,
                                       unsigned __int64 *rows_length) {
    unsigned char label[16];
    unsigned __int64 label_length = format_int(table_id, label, sizeof(label));
    unsigned __int64 schema_start;
    if (label_length == 0 || !app_data_expect_bytes(data, length, index, "TABLE", 5) ||
        !app_data_expect_bytes(data, length, index, (const char *)label, label_length) ||
        !app_data_expect_bytes(data, length, index, ":", 1) ||
        !app_data_read_uint_until(data, length, index, ':', schema_length) ||
        !app_data_read_uint_until(data, length, index, '\n', rows_length) ||
        *schema_length > length - *index) return 0;
    schema_start = *index;
    *schema = data + schema_start;
    *index += *schema_length;
    if (*rows_length > length - *index) return 0;
    *rows = data + *index;
    *index += *rows_length;
    return 1;
}

int app_data_load(const char *path_data, unsigned __int64 path_length) {
    static unsigned char document[JADREN_APP_DATA_DOCUMENT_MAX];
    static JadrenAppStateEntry parsed_state[JADREN_APP_STATE_MAX_ENTRIES];
    static JadrenAppList parsed_lists[JADREN_APP_LIST_MAX_LISTS];
    static JadrenAppTable parsed_tables[JADREN_APP_TABLE_MAX_TABLES];
    unsigned __int64 document_length;
    unsigned __int64 index;
    const unsigned char *payload;
    unsigned __int64 payload_length;
    int list_id;
    int table_id;
    if (path_data == 0 || path_length == 0 ||
        jadren_app_state_transaction_active || jadren_app_data_transaction_active ||
        jadren_app_table_transaction_active ||
        file_size(path_data, path_length) > JADREN_APP_DATA_DOCUMENT_MAX) return 0;
    document_length = file_read(path_data, path_length, document, sizeof(document));
    index = 0;
    if (!app_data_expect_bytes(document, document_length, &index,
                               "JADREN-APP-DATA-0.1\n", 20) ||
        !app_data_read_segment(document, document_length, &index, "STATE", 5,
                               &payload, &payload_length)) return 0;
    app_state_clear_entries(parsed_state);
    if (!app_state_parse_document(payload, payload_length, parsed_state)) return 0;
    for (list_id = 0; list_id < JADREN_APP_LIST_MAX_LISTS; list_id += 1) {
        const char *label = list_id == 0 ? "LIST0" : list_id == 1 ? "LIST1" : list_id == 2 ? "LIST2" : "LIST3";
        if (!app_data_read_segment(document, document_length, &index, label, 5,
                                   &payload, &payload_length)) return 0;
        app_list_clear_store(&parsed_lists[list_id]);
        if (!app_list_parse_document(payload, payload_length, &parsed_lists[list_id])) return 0;
    }
    for (table_id = 0; table_id < JADREN_APP_TABLE_MAX_TABLES; table_id += 1) {
        const unsigned char *schema;
        const unsigned char *rows;
        unsigned __int64 schema_length;
        unsigned __int64 rows_length;
        app_table_copy_schema(&parsed_tables[table_id], &jadren_app_tables[table_id]);
        app_table_clear_store(&parsed_tables[table_id]);
        if (!app_data_read_table_segment(document, document_length, &index, table_id,
                                         &schema, &schema_length, &rows, &rows_length) ||
            !app_table_parse_schema_full_document(schema, schema_length,
                                                  parsed_tables[table_id].column_types,
                                                  parsed_tables[table_id].column_names,
                                                  &parsed_tables[table_id].schema_version) ||
            !app_table_parse_document(rows, rows_length, &parsed_tables[table_id]) ||
            !app_table_store_valid(&parsed_tables[table_id])) return 0;
    }
    if (index != document_length) return 0;
    for (unsigned int entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1)
        app_state_copy_entry(&jadren_app_state[entry_index], &parsed_state[entry_index]);
    for (list_id = 0; list_id < JADREN_APP_LIST_MAX_LISTS; list_id += 1)
        app_list_copy_store(&jadren_app_lists[list_id], &parsed_lists[list_id]);
    for (table_id = 0; table_id < JADREN_APP_TABLE_MAX_TABLES; table_id += 1)
        app_table_copy_store(&jadren_app_tables[table_id], &parsed_tables[table_id]);
    app_state_bump_revision();
    app_data_refresh_bound_ui();
    return 1;
}

/* Load one complete caller-owned model document transactionally. The parser
 * fills bounded temporary stores first; malformed or truncated input leaves
 * the live state, lists and tables untouched. */
int app_data_load_exact(const unsigned char *input_data,
                        unsigned __int64 input_capacity,
                        unsigned __int64 input_length) {
    static JadrenAppStateEntry parsed_state[JADREN_APP_STATE_MAX_ENTRIES];
    static JadrenAppList parsed_lists[JADREN_APP_LIST_MAX_LISTS];
    static JadrenAppTable parsed_tables[JADREN_APP_TABLE_MAX_TABLES];
    unsigned __int64 index;
    const unsigned char *payload;
    unsigned __int64 payload_length;
    int list_id;
    int table_id;
    if (input_data == 0 || input_length == 0 || input_length > input_capacity ||
        input_length > JADREN_APP_DATA_DOCUMENT_MAX ||
        jadren_app_state_transaction_active || jadren_app_data_transaction_active ||
        jadren_app_table_transaction_active) return 0;
    index = 0;
    if (!app_data_expect_bytes(input_data, input_length, &index,
                               "JADREN-APP-DATA-0.1\n", 20) ||
        !app_data_read_segment(input_data, input_length, &index, "STATE", 5,
                               &payload, &payload_length)) return 0;
    app_state_clear_entries(parsed_state);
    if (!app_state_parse_document(payload, payload_length, parsed_state)) return 0;
    for (list_id = 0; list_id < JADREN_APP_LIST_MAX_LISTS; list_id += 1) {
        const char *label = list_id == 0 ? "LIST0" : list_id == 1 ? "LIST1" : list_id == 2 ? "LIST2" : "LIST3";
        if (!app_data_read_segment(input_data, input_length, &index, label, 5,
                                   &payload, &payload_length)) return 0;
        app_list_clear_store(&parsed_lists[list_id]);
        if (!app_list_parse_document(payload, payload_length, &parsed_lists[list_id])) return 0;
    }
    for (table_id = 0; table_id < JADREN_APP_TABLE_MAX_TABLES; table_id += 1) {
        const unsigned char *schema;
        const unsigned char *rows;
        unsigned __int64 schema_length;
        unsigned __int64 rows_length;
        app_table_copy_schema(&parsed_tables[table_id], &jadren_app_tables[table_id]);
        app_table_clear_store(&parsed_tables[table_id]);
        if (!app_data_read_table_segment(input_data, input_length, &index, table_id,
                                         &schema, &schema_length, &rows, &rows_length) ||
            !app_table_parse_schema_full_document(schema, schema_length,
                                                  parsed_tables[table_id].column_types,
                                                  parsed_tables[table_id].column_names,
                                                  &parsed_tables[table_id].schema_version) ||
            !app_table_parse_document(rows, rows_length, &parsed_tables[table_id]) ||
            !app_table_store_valid(&parsed_tables[table_id])) return 0;
    }
    if (index != input_length) return 0;
    for (unsigned int entry_index = 0; entry_index < JADREN_APP_STATE_MAX_ENTRIES; entry_index += 1)
        app_state_copy_entry(&jadren_app_state[entry_index], &parsed_state[entry_index]);
    for (list_id = 0; list_id < JADREN_APP_LIST_MAX_LISTS; list_id += 1)
        app_list_copy_store(&jadren_app_lists[list_id], &parsed_lists[list_id]);
    for (table_id = 0; table_id < JADREN_APP_TABLE_MAX_TABLES; table_id += 1)
        app_table_copy_store(&jadren_app_tables[table_id], &parsed_tables[table_id]);
    app_state_bump_revision();
    app_data_refresh_bound_ui();
    return 1;
}

/* Load a caller-owned model document only when the live model still matches
 * an equality-only snapshot token. This guard is process-local and keeps a
 * stale response from replacing newer local state. */
int app_data_load_exact_if_revision(const unsigned char *input_data,
                                    unsigned __int64 input_capacity,
                                    unsigned __int64 input_length,
                                    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_data_load_exact(input_data, input_capacity, input_length);
}

/*
 * Bounded append-only checkpoint journal.  New frames are:
 *   JADREN-APP-JOURNAL-0.2\n<decimal byte length>\n<decimal FNV-1a checksum>\n<payload>
 * Recovery also accepts the original 0.1 length-only frames. A torn or
 * checksum-invalid final frame is ignored; the last complete frame is written
 * to the caller-provided scratch path and loaded transactionally by
 * app_data_load. This remains process-local and does not claim fsync or
 * cross-process locking.
 */
#define JADREN_APP_JOURNAL_DOCUMENT_MAX (JADREN_APP_DATA_DOCUMENT_MAX * 4ULL)

static unsigned __int64 app_data_journal_checksum(const unsigned char *data,
                                                  unsigned __int64 length) {
    unsigned __int64 hash = 1469598103934665603ULL;
    unsigned __int64 index;
    if (data == 0 && length > 0) return 0;
    for (index = 0; index < length; index += 1) {
        hash ^= (unsigned __int64)data[index];
        hash *= 1099511628211ULL;
    }
    return hash;
}

/* Count only complete, checksum-valid journal frames.  The parser accepts
 * legacy 0.1 length-only frames and current 0.2 checksum frames, then stops
 * at the first torn or invalid tail. */
static unsigned __int64 app_data_journal_valid_frame_count(
    const char *journal_path, unsigned __int64 journal_path_length) {
    static unsigned char journal[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    const char marker_v1[] = "JADREN-APP-JOURNAL-0.1\n";
    const char marker_v2[] = "JADREN-APP-JOURNAL-0.2\n";
    unsigned __int64 journal_length;
    unsigned __int64 index = 0;
    unsigned __int64 count = 0;
    if (journal_path == 0 || journal_path_length == 0 ||
        (journal_length = file_size(journal_path, journal_path_length)) == 0 ||
        journal_length > sizeof(journal) ||
        file_read(journal_path, journal_path_length, journal, journal_length) != journal_length) {
        return 0;
    }
    while (index < journal_length) {
        unsigned __int64 parsed_length = 0;
        unsigned __int64 parsed_checksum = 0;
        unsigned __int64 digits_start;
        unsigned __int64 digit;
        unsigned __int64 marker_length = 0;
        int has_checksum = 0;
        if (sizeof(marker_v2) - 1 <= journal_length - index) {
            for (digit = 0; digit < sizeof(marker_v2) - 1; digit += 1)
                if (journal[index + digit] != (unsigned char)marker_v2[digit]) break;
            if (digit == sizeof(marker_v2) - 1) {
                marker_length = sizeof(marker_v2) - 1;
                has_checksum = 1;
            }
        }
        if (marker_length == 0 && sizeof(marker_v1) - 1 <= journal_length - index) {
            for (digit = 0; digit < sizeof(marker_v1) - 1; digit += 1)
                if (journal[index + digit] != (unsigned char)marker_v1[digit]) break;
            if (digit == sizeof(marker_v1) - 1) marker_length = sizeof(marker_v1) - 1;
        }
        if (marker_length == 0) break;
        index += marker_length;
        digits_start = index;
        while (index < journal_length && journal[index] != (unsigned char)'\n') {
            unsigned char value = journal[index];
            if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                parsed_length > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10)
                return count;
            parsed_length = parsed_length * 10 + (value - (unsigned char)'0');
            index += 1;
        }
        if (index == digits_start || index >= journal_length || parsed_length == 0) break;
        index += 1;
        if (has_checksum) {
            digits_start = index;
            while (index < journal_length && journal[index] != (unsigned char)'\n') {
                unsigned char value = journal[index];
                if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                    parsed_checksum > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10)
                    return count;
                parsed_checksum = parsed_checksum * 10 + (value - (unsigned char)'0');
                index += 1;
            }
            if (index == digits_start || index >= journal_length) break;
            index += 1;
        }
        if (parsed_length > journal_length - index ||
            (has_checksum && app_data_journal_checksum(journal + index, parsed_length) != parsed_checksum))
            break;
        count += 1;
        index += parsed_length;
    }
    return count;
}

/* Return the number of complete valid frames while holding the caller-owned
 * lock, so replay callers can discover the valid zero-based index range. */
unsigned __int64 app_data_journal_count_frames_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *lock_path, unsigned __int64 lock_path_length) {
    unsigned __int64 lock_token;
    unsigned __int64 count;
    if (journal_path == 0 || lock_path == 0 || journal_path_length == 0 || lock_path_length == 0)
        return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    count = app_data_journal_valid_frame_count(journal_path, journal_path_length);
    if (!file_unlock(lock_token)) return 0;
    return count;
}

/* Return the valid frame count and on-disk byte size from one lock-held
 * journal snapshot. output[0] is the count and output[1] is the byte size. */
int app_data_journal_stats_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 *output_data, unsigned __int64 output_length) {
    unsigned __int64 lock_token;
    unsigned __int64 size;
    unsigned __int64 count;
    if (journal_path == 0 || lock_path == 0 || journal_path_length == 0 ||
        lock_path_length == 0 || output_data == 0 || output_length < 2)
        return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    if (!file_exists(journal_path, journal_path_length)) {
        file_unlock(lock_token);
        return 0;
    }
    size = file_size(journal_path, journal_path_length);
    count = app_data_journal_valid_frame_count(journal_path, journal_path_length);
    output_data[0] = count;
    output_data[1] = size;
    if (!file_unlock(lock_token)) return 0;
    return 1;
}

/* Produce a caller-owned maintenance plan from one lock-held snapshot.
 * output[0]: 0 = no action, 1 = canonical compaction, 2 = retain frames;
 * output[1] is the complete frame count and output[2] is journal byte size. */
int app_data_journal_maintenance_plan_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 max_bytes, unsigned __int64 max_frames,
    unsigned __int64 *output_data, unsigned __int64 output_length) {
    unsigned __int64 lock_token;
    unsigned __int64 journal_size;
    unsigned __int64 frame_count;
    unsigned __int64 action;
    if (journal_path == 0 || lock_path == 0 || journal_path_length == 0 ||
        lock_path_length == 0 || max_bytes == 0 || max_frames == 0 ||
        output_data == 0 || output_length < 3) return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    if (!file_exists(journal_path, journal_path_length)) {
        file_unlock(lock_token);
        return 0;
    }
    journal_size = file_size(journal_path, journal_path_length);
    frame_count = app_data_journal_valid_frame_count(journal_path, journal_path_length);
    action = frame_count > max_frames ? 2 : (journal_size > max_bytes ? 1 : 0);
    if (!file_unlock(lock_token)) return 0;
    output_data[0] = action;
    output_data[1] = frame_count;
    output_data[2] = journal_size;
    return 1;
}

/* Read one zero-based complete frame into caller-owned memory while holding
 * the lock. Invalid or torn tails are ignored; short output is rejected
 * before any output byte or length slot is changed. */
static int app_data_journal_read_frame_exact(
    const char *journal_path, unsigned __int64 journal_path_length,
    unsigned __int64 frame_index, unsigned char *output_data,
    unsigned __int64 output_length, unsigned __int64 *written_data,
    unsigned __int64 written_capacity, unsigned __int64 *frame_offset_data,
    unsigned __int64 *frame_total_data) {
    static unsigned char journal[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    const char marker_v1[] = "JADREN-APP-JOURNAL-0.1\n";
    const char marker_v2[] = "JADREN-APP-JOURNAL-0.2\n";
    unsigned __int64 journal_length;
    unsigned __int64 index = 0;
    unsigned __int64 current_index = 0;
    if (journal_path == 0 || journal_path_length == 0 ||
        ((output_data == 0) != (output_length == 0)) ||
        written_data == 0 || written_capacity == 0 ||
        (journal_length = file_size(journal_path, journal_path_length)) == 0 ||
        journal_length > sizeof(journal) ||
        file_read(journal_path, journal_path_length, journal, journal_length) != journal_length)
        return 0;
    while (index < journal_length) {
        unsigned __int64 frame_start = index;
        unsigned __int64 parsed_length = 0;
        unsigned __int64 parsed_checksum = 0;
        unsigned __int64 digits_start;
        unsigned __int64 digit;
        unsigned __int64 marker_length = 0;
        int has_checksum = 0;
        if (sizeof(marker_v2) - 1 <= journal_length - index) {
            for (digit = 0; digit < sizeof(marker_v2) - 1; digit += 1)
                if (journal[index + digit] != (unsigned char)marker_v2[digit]) break;
            if (digit == sizeof(marker_v2) - 1) {
                marker_length = sizeof(marker_v2) - 1;
                has_checksum = 1;
            }
        }
        if (marker_length == 0 && sizeof(marker_v1) - 1 <= journal_length - index) {
            for (digit = 0; digit < sizeof(marker_v1) - 1; digit += 1)
                if (journal[index + digit] != (unsigned char)marker_v1[digit]) break;
            if (digit == sizeof(marker_v1) - 1) marker_length = sizeof(marker_v1) - 1;
        }
        if (marker_length == 0) break;
        index += marker_length;
        digits_start = index;
        while (index < journal_length && journal[index] != (unsigned char)'\n') {
            unsigned char value = journal[index];
            if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                parsed_length > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10)
                return 0;
            parsed_length = parsed_length * 10 + (value - (unsigned char)'0');
            index += 1;
        }
        if (index == digits_start || index >= journal_length || parsed_length == 0) break;
        index += 1;
        if (has_checksum) {
            digits_start = index;
            while (index < journal_length && journal[index] != (unsigned char)'\n') {
                unsigned char value = journal[index];
                if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                    parsed_checksum > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10)
                    return 0;
                parsed_checksum = parsed_checksum * 10 + (value - (unsigned char)'0');
                index += 1;
            }
            if (index == digits_start || index >= journal_length) break;
            index += 1;
        }
        if (parsed_length > journal_length - index ||
            (has_checksum && app_data_journal_checksum(journal + index, parsed_length) != parsed_checksum))
            break;
        if (current_index == frame_index) {
            if (frame_offset_data != 0) *frame_offset_data = frame_start;
            if (frame_total_data != 0) *frame_total_data =
                (index + parsed_length) - frame_start;
            if (output_data == 0) {
                written_data[0] = parsed_length;
                return 1;
            }
            if (parsed_length > output_length) return 0;
            for (digit = 0; digit < parsed_length; digit += 1)
                output_data[digit] = journal[index + digit];
            written_data[0] = parsed_length;
            return 1;
        }
        current_index += 1;
        index += parsed_length;
    }
    return 0;
}

/* Return one complete frame length without allocating or copying a payload.
 * The null/zero output mode is private to this preflight helper; the public
 * read API still requires a caller-owned output buffer. */
static unsigned __int64 app_data_journal_frame_length(
    const char *journal_path, unsigned __int64 journal_path_length,
    unsigned __int64 frame_index) {
    unsigned __int64 written = 0;
    if (!app_data_journal_read_frame_exact(
            journal_path, journal_path_length, frame_index, 0, 0,
            &written, 1, 0, 0)) return 0;
    return written;
}

/* Read the newest complete valid frame while the caller-owned lock remains
 * held. Counting and copying share one lock, so retention cannot change the
 * selected frame between the two operations. */
static int app_data_journal_read_latest_frame_exact(
    const char *journal_path, unsigned __int64 journal_path_length,
    unsigned char *output_data, unsigned __int64 output_length,
    unsigned __int64 *written_data, unsigned __int64 written_capacity) {
    unsigned __int64 count = app_data_journal_valid_frame_count(
        journal_path, journal_path_length);
    if (count == 0) return 0;
    return app_data_journal_read_frame_exact(
        journal_path, journal_path_length, count - 1, output_data,
        output_length, written_data, written_capacity, 0, 0);
}

int app_data_journal_read_frame_exact_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 frame_index, unsigned char *output_data,
    unsigned __int64 output_length, unsigned __int64 *written_data,
    unsigned __int64 written_capacity) {
    unsigned __int64 lock_token;
    int read;
    if (journal_path == 0 || lock_path == 0 || journal_path_length == 0 ||
        lock_path_length == 0 || output_data == 0 || output_length == 0 ||
        written_data == 0 || written_capacity == 0)
        return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    read = app_data_journal_read_frame_exact(
        journal_path, journal_path_length, frame_index, output_data,
        output_length, written_data, written_capacity, 0, 0);
    if (!file_unlock(lock_token)) return 0;
    return read;
}

int app_data_journal_read_latest_frame_exact_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned char *output_data, unsigned __int64 output_length,
    unsigned __int64 *written_data, unsigned __int64 written_capacity) {
    unsigned __int64 lock_token;
    int read;
    if (journal_path == 0 || lock_path == 0 || journal_path_length == 0 ||
        lock_path_length == 0 || output_data == 0 || output_length == 0 ||
        written_data == 0 || written_capacity == 0)
        return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    read = app_data_journal_read_latest_frame_exact(
        journal_path, journal_path_length, output_data, output_length,
        written_data, written_capacity);
    if (!file_unlock(lock_token)) return 0;
    return read;
}

/* Discover one complete payload length under the caller-owned lock. A zero
 * result means invalid/missing/torn input or an out-of-range frame index. */
unsigned __int64 app_data_journal_frame_length_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 frame_index) {
    unsigned __int64 lock_token;
    unsigned __int64 length;
    if (journal_path == 0 || lock_path == 0 || journal_path_length == 0 ||
        lock_path_length == 0) return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    length = app_data_journal_frame_length(
        journal_path, journal_path_length, frame_index);
    if (!file_unlock(lock_token)) return 0;
    return length;
}

/* Return [frame_start_offset, frame_total_bytes, payload_bytes] for one
 * complete checksum-valid frame from a single lock-held snapshot. */
int app_data_journal_frame_span_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 frame_index, unsigned __int64 *output_data,
    unsigned __int64 output_length) {
    unsigned __int64 lock_token;
    unsigned __int64 frame_offset = 0;
    unsigned __int64 frame_total = 0;
    unsigned __int64 payload_length = 0;
    int read;
    if (journal_path == 0 || lock_path == 0 || journal_path_length == 0 ||
        lock_path_length == 0 || output_data == 0 || output_length < 3)
        return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    read = app_data_journal_read_frame_exact(
        journal_path, journal_path_length, frame_index, 0, 0,
        &payload_length, 1, &frame_offset, &frame_total);
    if (!file_unlock(lock_token) || !read) return 0;
    output_data[0] = frame_offset;
    output_data[1] = frame_total;
    output_data[2] = payload_length;
    return 1;
}

/* Build a compact persistent index for the current journal snapshot. The
 * index header stores journal byte size and checksum, followed by fixed
 * 24-byte [offset, total_bytes, payload_bytes] records. */
static void app_data_journal_index_put_u64(
    unsigned char *data, unsigned __int64 offset, unsigned __int64 value) {
    unsigned int digit;
    for (digit = 0; digit < 8; digit += 1) {
        data[offset + digit] = (unsigned char)(value & 0xFFULL);
        value >>= 8;
    }
}

static unsigned __int64 app_data_journal_index_get_u64(
    const unsigned char *data, unsigned __int64 offset) {
    unsigned __int64 value = 0;
    int digit;
    for (digit = 7; digit >= 0; digit -= 1)
        value = (value << 8) | (unsigned __int64)data[offset + (unsigned int)digit];
    return value;
}

static int app_data_journal_build_index_document(
    const char *journal_path, unsigned __int64 journal_path_length,
    unsigned char *index_data, unsigned __int64 index_capacity,
    unsigned __int64 *index_length_data) {
    static unsigned char journal[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    static const char magic[] = "JADREN-APP-JINDEX-0.1\n";
    const char marker_v1[] = "JADREN-APP-JOURNAL-0.1\n";
    const char marker_v2[] = "JADREN-APP-JOURNAL-0.2\n";
    unsigned __int64 journal_length;
    unsigned __int64 index_offset = sizeof(magic) - 1;
    unsigned __int64 count_offset = index_offset + 16;
    unsigned __int64 frame_count = 0;
    unsigned __int64 index = 0;
    if (journal_path == 0 || journal_path_length == 0 || index_data == 0 ||
        index_length_data == 0 || index_capacity < sizeof(magic) - 1 + 24 ||
        (journal_length = file_size(journal_path, journal_path_length)) == 0 ||
        journal_length > sizeof(journal) ||
        file_read(journal_path, journal_path_length, journal, journal_length) != journal_length)
        return 0;
    for (unsigned __int64 digit = 0; digit < sizeof(magic) - 1; digit += 1)
        index_data[digit] = (unsigned char)magic[digit];
    app_data_journal_index_put_u64(index_data, index_offset, journal_length);
    index_offset += 8;
    app_data_journal_index_put_u64(index_data, index_offset,
                                   app_data_journal_checksum(journal, journal_length));
    index_offset += 8;
    app_data_journal_index_put_u64(index_data, index_offset, 0);
    count_offset = index_offset;
    index_offset += 8;
    while (index < journal_length) {
        unsigned __int64 frame_start = index;
        unsigned __int64 parsed_length = 0;
        unsigned __int64 parsed_checksum = 0;
        unsigned __int64 digits_start;
        unsigned __int64 digit;
        unsigned __int64 marker_length = 0;
        int has_checksum = 0;
        int malformed = 0;
        if (sizeof(marker_v2) - 1 <= journal_length - index) {
            for (digit = 0; digit < sizeof(marker_v2) - 1; digit += 1)
                if (journal[index + digit] != (unsigned char)marker_v2[digit]) break;
            if (digit == sizeof(marker_v2) - 1) {
                marker_length = sizeof(marker_v2) - 1;
                has_checksum = 1;
            }
        }
        if (marker_length == 0 && sizeof(marker_v1) - 1 <= journal_length - index) {
            for (digit = 0; digit < sizeof(marker_v1) - 1; digit += 1)
                if (journal[index + digit] != (unsigned char)marker_v1[digit]) break;
            if (digit == sizeof(marker_v1) - 1) marker_length = sizeof(marker_v1) - 1;
        }
        if (marker_length == 0) break;
        index += marker_length;
        digits_start = index;
        while (index < journal_length && journal[index] != (unsigned char)'\n') {
            unsigned char value = journal[index];
            if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                parsed_length > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10) {
                malformed = 1;
                break;
            }
            parsed_length = parsed_length * 10 + (value - (unsigned char)'0');
            index += 1;
        }
        if (malformed || index == digits_start || index >= journal_length || parsed_length == 0) break;
        index += 1;
        if (has_checksum) {
            digits_start = index;
            while (index < journal_length && journal[index] != (unsigned char)'\n') {
                unsigned char value = journal[index];
                if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                    parsed_checksum > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10) {
                    malformed = 1;
                    break;
                }
                parsed_checksum = parsed_checksum * 10 + (value - (unsigned char)'0');
                index += 1;
            }
            if (malformed || index == digits_start || index >= journal_length) break;
            index += 1;
        }
        if (parsed_length > journal_length - index ||
            (has_checksum && app_data_journal_checksum(journal + index, parsed_length) != parsed_checksum))
            break;
        if (index_offset > index_capacity || index_capacity - index_offset < 24) return 0;
        app_data_journal_index_put_u64(index_data, index_offset, frame_start);
        app_data_journal_index_put_u64(index_data, index_offset + 8,
                                       (index + parsed_length) - frame_start);
        app_data_journal_index_put_u64(index_data, index_offset + 16, parsed_length);
        index_offset += 24;
        frame_count += 1;
        index += parsed_length;
    }
    app_data_journal_index_put_u64(index_data, count_offset, frame_count);
    *index_length_data = index_offset;
    return 1;
}

int app_data_journal_build_index_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *index_path, unsigned __int64 index_path_length,
    const char *temporary_path, unsigned __int64 temporary_path_length,
    const char *lock_path, unsigned __int64 lock_path_length) {
    static unsigned char index_data[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    unsigned __int64 lock_token;
    unsigned __int64 index_length;
    int success = 0;
    if (journal_path == 0 || index_path == 0 || temporary_path == 0 || lock_path == 0 ||
        journal_path_length == 0 || index_path_length == 0 || temporary_path_length == 0 ||
        lock_path_length == 0) return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    if (app_data_journal_build_index_document(
            journal_path, journal_path_length, index_data, sizeof(index_data), &index_length) &&
        file_write(temporary_path, temporary_path_length, index_data, index_length) == index_length &&
        file_flush(temporary_path, temporary_path_length) &&
        file_replace_atomic(temporary_path, temporary_path_length, index_path, index_path_length) &&
        file_flush(index_path, index_path_length)) success = 1;
    if (!success) (void)file_delete(temporary_path, temporary_path_length);
    if (!file_unlock(lock_token)) return 0;
    return success;
}

int app_data_journal_index_lookup_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *index_path, unsigned __int64 index_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 frame_index, unsigned __int64 *output_data,
    unsigned __int64 output_length) {
    static unsigned char journal[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    static unsigned char index_data[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    static const char magic[] = "JADREN-APP-JINDEX-0.1\n";
    unsigned __int64 lock_token;
    unsigned __int64 journal_length;
    unsigned __int64 index_length;
    unsigned __int64 header_length = sizeof(magic) - 1 + 24;
    unsigned __int64 indexed_size;
    unsigned __int64 indexed_checksum;
    unsigned __int64 frame_count;
    unsigned __int64 entry_offset;
    unsigned __int64 frame_offset = 0;
    unsigned __int64 frame_total = 0;
    unsigned __int64 payload_length = 0;
    unsigned __int64 previous_offset = 0;
    unsigned __int64 entry_index;
    if (journal_path == 0 || index_path == 0 || lock_path == 0 || output_data == 0 ||
        journal_path_length == 0 || index_path_length == 0 || lock_path_length == 0 ||
        output_length < 3) return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    journal_length = file_size(journal_path, journal_path_length);
    index_length = file_size(index_path, index_path_length);
    if (journal_length == 0 || journal_length > sizeof(journal) || index_length < header_length ||
        index_length > sizeof(index_data) ||
        file_read(journal_path, journal_path_length, journal, journal_length) != journal_length ||
        file_read(index_path, index_path_length, index_data, index_length) != index_length)
        goto index_lookup_done;
    for (entry_index = 0; entry_index < sizeof(magic) - 1; entry_index += 1)
        if (index_data[entry_index] != (unsigned char)magic[entry_index]) goto index_lookup_done;
    indexed_size = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1);
    indexed_checksum = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1 + 8);
    frame_count = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1 + 16);
    if (indexed_size != journal_length ||
        indexed_checksum != app_data_journal_checksum(journal, journal_length) ||
        frame_count > (index_length - header_length) / 24 ||
        header_length + frame_count * 24 != index_length || frame_index >= frame_count)
        goto index_lookup_done;
    entry_offset = header_length;
    for (entry_index = 0; entry_index < frame_count; entry_index += 1) {
        unsigned __int64 current_offset = app_data_journal_index_get_u64(index_data, entry_offset);
        unsigned __int64 current_total = app_data_journal_index_get_u64(index_data, entry_offset + 8);
        unsigned __int64 current_payload = app_data_journal_index_get_u64(index_data, entry_offset + 16);
        if (current_payload == 0 || current_total <= current_payload || current_offset > journal_length ||
            current_total > journal_length - current_offset ||
            (entry_index > 0 && current_offset <= previous_offset)) goto index_lookup_done;
        if (entry_index == frame_index) {
            frame_offset = current_offset;
            frame_total = current_total;
            payload_length = current_payload;
        }
        previous_offset = current_offset;
        entry_offset += 24;
    }
    if (!file_unlock(lock_token)) return 0;
    output_data[0] = frame_offset;
    output_data[1] = frame_total;
    output_data[2] = payload_length;
    return 1;
index_lookup_done:
    (void)file_unlock(lock_token);
    return 0;
}

static int app_data_journal_index_csv_add_size(
    unsigned __int64 *total, unsigned __int64 amount) {
    if (total == 0 || *total > ((unsigned __int64)-1) - amount) return 0;
    *total += amount;
    return 1;
}

/* Export all indexed spans as numeric CSV without copying journal payloads.
 * The index snapshot is validated before any caller-owned output byte changes. */
int app_data_journal_index_export_csv_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *index_path, unsigned __int64 index_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned char *output_data, unsigned __int64 output_length) {
    static unsigned char journal[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    static unsigned char index_data[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    static const char magic[] = "JADREN-APP-JINDEX-0.1\n";
    static const char header[] = "frame_index,offset,total_bytes,payload_bytes\n";
    unsigned __int64 lock_token;
    unsigned __int64 journal_length;
    unsigned __int64 index_length;
    unsigned __int64 header_length = sizeof(magic) - 1 + 24;
    unsigned __int64 indexed_size;
    unsigned __int64 indexed_checksum;
    unsigned __int64 frame_count;
    unsigned __int64 entry_offset;
    unsigned __int64 previous_offset = 0;
    unsigned __int64 required = sizeof(header) - 1;
    unsigned __int64 written = 0;
    unsigned __int64 entry_index;
    unsigned char digits[32];
    if (journal_path == 0 || index_path == 0 || lock_path == 0 || output_data == 0 ||
        journal_path_length == 0 || index_path_length == 0 || lock_path_length == 0)
        return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    journal_length = file_size(journal_path, journal_path_length);
    index_length = file_size(index_path, index_path_length);
    if (journal_length == 0 || journal_length > sizeof(journal) || index_length < header_length ||
        index_length > sizeof(index_data) ||
        file_read(journal_path, journal_path_length, journal, journal_length) != journal_length ||
        file_read(index_path, index_path_length, index_data, index_length) != index_length)
        goto index_export_done;
    for (entry_index = 0; entry_index < sizeof(magic) - 1; entry_index += 1)
        if (index_data[entry_index] != (unsigned char)magic[entry_index]) goto index_export_done;
    indexed_size = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1);
    indexed_checksum = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1 + 8);
    frame_count = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1 + 16);
    if (indexed_size != journal_length ||
        indexed_checksum != app_data_journal_checksum(journal, journal_length) ||
        frame_count > (index_length - header_length) / 24 ||
        header_length + frame_count * 24 != index_length)
        goto index_export_done;
    entry_offset = header_length;
    for (entry_index = 0; entry_index < frame_count; entry_index += 1) {
        unsigned __int64 current_offset = app_data_journal_index_get_u64(index_data, entry_offset);
        unsigned __int64 current_total = app_data_journal_index_get_u64(index_data, entry_offset + 8);
        unsigned __int64 current_payload = app_data_journal_index_get_u64(index_data, entry_offset + 16);
        unsigned __int64 number_length;
        if (current_payload == 0 || current_total <= current_payload || current_offset > journal_length ||
            current_total > journal_length - current_offset ||
            (entry_index > 0 && current_offset <= previous_offset)) goto index_export_done;
        number_length = format_uint(entry_index, digits, sizeof(digits));
        if (!app_data_journal_index_csv_add_size(&required, number_length) ||
            !app_data_journal_index_csv_add_size(&required, 1)) goto index_export_done;
        number_length = format_uint(current_offset, digits, sizeof(digits));
        if (!app_data_journal_index_csv_add_size(&required, number_length) ||
            !app_data_journal_index_csv_add_size(&required, 1)) goto index_export_done;
        number_length = format_uint(current_total, digits, sizeof(digits));
        if (!app_data_journal_index_csv_add_size(&required, number_length) ||
            !app_data_journal_index_csv_add_size(&required, 1)) goto index_export_done;
        number_length = format_uint(current_payload, digits, sizeof(digits));
        if (!app_data_journal_index_csv_add_size(&required, number_length) ||
            !app_data_journal_index_csv_add_size(&required, 1)) goto index_export_done;
        previous_offset = current_offset;
        entry_offset += 24;
    }
    if (output_length < required) goto index_export_done;
    for (entry_index = 0; entry_index < sizeof(header) - 1; entry_index += 1)
        output_data[written++] = (unsigned char)header[entry_index];
    entry_offset = header_length;
    for (entry_index = 0; entry_index < frame_count; entry_index += 1) {
        unsigned __int64 current_offset = app_data_journal_index_get_u64(index_data, entry_offset);
        unsigned __int64 current_total = app_data_journal_index_get_u64(index_data, entry_offset + 8);
        unsigned __int64 current_payload = app_data_journal_index_get_u64(index_data, entry_offset + 16);
        unsigned __int64 number_length;
        unsigned __int64 digit;
        number_length = format_uint(entry_index, digits, sizeof(digits));
        for (digit = 0; digit < number_length; digit += 1) output_data[written++] = digits[digit];
        output_data[written++] = (unsigned char)',';
        number_length = format_uint(current_offset, digits, sizeof(digits));
        for (digit = 0; digit < number_length; digit += 1) output_data[written++] = digits[digit];
        output_data[written++] = (unsigned char)',';
        number_length = format_uint(current_total, digits, sizeof(digits));
        for (digit = 0; digit < number_length; digit += 1) output_data[written++] = digits[digit];
        output_data[written++] = (unsigned char)',';
        number_length = format_uint(current_payload, digits, sizeof(digits));
        for (digit = 0; digit < number_length; digit += 1) output_data[written++] = digits[digit];
        output_data[written++] = (unsigned char)'\n';
        entry_offset += 24;
    }
    if (!file_unlock(lock_token)) return 0;
    return written == required ? written : 0;
index_export_done:
    (void)file_unlock(lock_token);
    return 0;
}

static int app_data_journal_export_path_same(
    const char *first, unsigned __int64 first_length,
    const char *second, unsigned __int64 second_length) {
    unsigned __int64 index;
    if (first == 0 || second == 0 || first_length != second_length) return 0;
    for (index = 0; index < first_length; index += 1)
        if (first[index] != second[index]) return 0;
    return 1;
}

static unsigned __int64 app_data_journal_index_csv_row(
    unsigned __int64 frame_index, unsigned __int64 frame_offset,
    unsigned __int64 frame_total, unsigned __int64 payload_length,
    unsigned char *output_data, unsigned __int64 output_length) {
    unsigned char digits[32];
    unsigned __int64 written = 0;
    unsigned __int64 number_length;
    unsigned __int64 digit;
    if (output_data == 0 || output_length == 0) return 0;
    number_length = format_uint(frame_index, digits, sizeof(digits));
    if (number_length == 0 || written > output_length ||
        output_length - written < number_length + 1) return 0;
    for (digit = 0; digit < number_length; digit += 1) output_data[written++] = digits[digit];
    output_data[written++] = (unsigned char)',';
    number_length = format_uint(frame_offset, digits, sizeof(digits));
    if (number_length == 0 || output_length - written < number_length + 1) return 0;
    for (digit = 0; digit < number_length; digit += 1) output_data[written++] = digits[digit];
    output_data[written++] = (unsigned char)',';
    number_length = format_uint(frame_total, digits, sizeof(digits));
    if (number_length == 0 || output_length - written < number_length + 1) return 0;
    for (digit = 0; digit < number_length; digit += 1) output_data[written++] = digits[digit];
    output_data[written++] = (unsigned char)',';
    number_length = format_uint(payload_length, digits, sizeof(digits));
    if (number_length == 0 || output_length - written < number_length + 1) return 0;
    for (digit = 0; digit < number_length; digit += 1) output_data[written++] = digits[digit];
    output_data[written++] = (unsigned char)'\n';
    return written;
}

/* Stream the validated index to an atomically promoted CSV file. */
int app_data_journal_index_export_csv_file_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *index_path, unsigned __int64 index_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    const char *output_path, unsigned __int64 output_path_length,
    const char *temporary_path, unsigned __int64 temporary_path_length) {
    static unsigned char journal[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    static unsigned char index_data[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    static unsigned char row[128];
    static const char magic[] = "JADREN-APP-JINDEX-0.1\n";
    static const char header[] = "frame_index,offset,total_bytes,payload_bytes\n";
    unsigned __int64 lock_token;
    unsigned __int64 journal_length;
    unsigned __int64 index_length;
    unsigned __int64 header_length = sizeof(magic) - 1 + 24;
    unsigned __int64 indexed_size;
    unsigned __int64 indexed_checksum;
    unsigned __int64 frame_count;
    unsigned __int64 entry_offset;
    unsigned __int64 previous_offset = 0;
    unsigned __int64 entry_index;
    unsigned __int64 row_length;
    int success = 0;
    if (journal_path == 0 || index_path == 0 || lock_path == 0 ||
        output_path == 0 || temporary_path == 0 ||
        journal_path_length == 0 || index_path_length == 0 ||
        lock_path_length == 0 || output_path_length == 0 ||
        temporary_path_length == 0 ||
        app_data_journal_export_path_same(output_path, output_path_length,
                                          journal_path, journal_path_length) ||
        app_data_journal_export_path_same(output_path, output_path_length,
                                          index_path, index_path_length) ||
        app_data_journal_export_path_same(output_path, output_path_length,
                                          lock_path, lock_path_length) ||
        app_data_journal_export_path_same(temporary_path, temporary_path_length,
                                          journal_path, journal_path_length) ||
        app_data_journal_export_path_same(temporary_path, temporary_path_length,
                                          index_path, index_path_length) ||
        app_data_journal_export_path_same(temporary_path, temporary_path_length,
                                          lock_path, lock_path_length) ||
        app_data_journal_export_path_same(temporary_path, temporary_path_length,
                                          output_path, output_path_length))
        return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    journal_length = file_size(journal_path, journal_path_length);
    index_length = file_size(index_path, index_path_length);
    if (journal_length == 0 || journal_length > sizeof(journal) ||
        index_length < header_length || index_length > sizeof(index_data) ||
        file_read(journal_path, journal_path_length, journal, journal_length) != journal_length ||
        file_read(index_path, index_path_length, index_data, index_length) != index_length)
        goto index_file_export_done;
    for (entry_index = 0; entry_index < sizeof(magic) - 1; entry_index += 1)
        if (index_data[entry_index] != (unsigned char)magic[entry_index])
            goto index_file_export_done;
    indexed_size = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1);
    indexed_checksum = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1 + 8);
    frame_count = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1 + 16);
    if (indexed_size != journal_length ||
        indexed_checksum != app_data_journal_checksum(journal, journal_length) ||
        frame_count > (index_length - header_length) / 24 ||
        header_length + frame_count * 24 != index_length)
        goto index_file_export_done;
    entry_offset = header_length;
    for (entry_index = 0; entry_index < frame_count; entry_index += 1) {
        unsigned __int64 current_offset = app_data_journal_index_get_u64(index_data, entry_offset);
        unsigned __int64 current_total = app_data_journal_index_get_u64(index_data, entry_offset + 8);
        unsigned __int64 current_payload = app_data_journal_index_get_u64(index_data, entry_offset + 16);
        if (current_payload == 0 || current_total <= current_payload ||
            current_offset > journal_length || current_total > journal_length - current_offset ||
            (entry_index > 0 && current_offset <= previous_offset))
            goto index_file_export_done;
        previous_offset = current_offset;
        entry_offset += 24;
    }
    if (file_write(temporary_path, temporary_path_length,
                   (const unsigned char *)header, sizeof(header) - 1) != sizeof(header) - 1)
        goto index_file_export_done;
    entry_offset = header_length;
    for (entry_index = 0; entry_index < frame_count; entry_index += 1) {
        unsigned __int64 current_offset = app_data_journal_index_get_u64(index_data, entry_offset);
        unsigned __int64 current_total = app_data_journal_index_get_u64(index_data, entry_offset + 8);
        unsigned __int64 current_payload = app_data_journal_index_get_u64(index_data, entry_offset + 16);
        row_length = app_data_journal_index_csv_row(
            entry_index, current_offset, current_total, current_payload, row, sizeof(row));
        if (row_length == 0 ||
            file_append(temporary_path, temporary_path_length, row, row_length, row_length) != row_length)
            goto index_file_export_done;
        entry_offset += 24;
    }
    if (!file_flush(temporary_path, temporary_path_length) ||
        !file_replace_atomic(temporary_path, temporary_path_length,
                              output_path, output_path_length) ||
        !file_flush(output_path, output_path_length))
        goto index_file_export_done;
    success = 1;
index_file_export_done:
    if (!success) (void)file_delete(temporary_path, temporary_path_length);
    if (!file_unlock(lock_token)) return 0;
    return success;
}

/* Export a bounded list to a flushed temporary CSV and atomically promote it. */
int app_list_export_csv_file_durable(
    int list_id,
    const char *output_path_data, unsigned __int64 output_path_length,
    const char *temporary_path_data, unsigned __int64 temporary_path_length) {
    static unsigned char document[JADREN_APP_LIST_DOCUMENT_MAX];
    static const unsigned char empty_document[1] = {0};
    unsigned __int64 csv_length;
    int success = 0;
    if (!app_list_valid(list_id) || output_path_data == 0 || temporary_path_data == 0 ||
        output_path_length == 0 || temporary_path_length == 0 ||
        app_data_journal_export_path_same(output_path_data, output_path_length,
                                          temporary_path_data, temporary_path_length))
        return 0;
    if (file_exists(temporary_path_data, temporary_path_length) &&
        !file_delete(temporary_path_data, temporary_path_length))
        goto list_csv_file_done;
    if (jadren_app_lists[list_id].count == 0) {
        if (file_write(temporary_path_data, temporary_path_length,
                       empty_document, 0) != 0 ||
            !file_exists(temporary_path_data, temporary_path_length) ||
            directory_exists(temporary_path_data, temporary_path_length))
            goto list_csv_file_done;
    } else {
        csv_length = app_list_export_csv(list_id, document, sizeof(document));
        if (csv_length == 0 ||
            file_write(temporary_path_data, temporary_path_length,
                       document, csv_length) != csv_length)
            goto list_csv_file_done;
    }
    if (!file_flush(temporary_path_data, temporary_path_length) ||
        !file_replace_atomic(temporary_path_data, temporary_path_length,
                             output_path_data, output_path_length) ||
        !file_flush(output_path_data, output_path_length))
        goto list_csv_file_done;
    success = 1;
list_csv_file_done:
    if (!success) (void)file_delete(temporary_path_data, temporary_path_length);
    return success;
}

/* Export a bounded table to a flushed temporary CSV and atomically promote it. */
int app_table_export_csv_file_durable(
    int table_id,
    const char *output_path_data, unsigned __int64 output_path_length,
    const char *temporary_path_data, unsigned __int64 temporary_path_length) {
    static unsigned char document[JADREN_APP_TABLE_DOCUMENT_MAX];
    unsigned __int64 csv_length;
    int success = 0;
    if (!app_table_valid(table_id) || output_path_data == 0 || temporary_path_data == 0 ||
        output_path_length == 0 || temporary_path_length == 0 ||
        app_data_journal_export_path_same(output_path_data, output_path_length,
                                          temporary_path_data, temporary_path_length))
        return 0;
    if (file_exists(temporary_path_data, temporary_path_length) &&
        !file_delete(temporary_path_data, temporary_path_length))
        goto table_csv_file_done;
    csv_length = app_table_export_csv(table_id, document, sizeof(document));
    if (csv_length == 0 ||
        file_write(temporary_path_data, temporary_path_length,
                   document, csv_length) != csv_length)
        goto table_csv_file_done;
    if (!file_flush(temporary_path_data, temporary_path_length) ||
        !file_replace_atomic(temporary_path_data, temporary_path_length,
                             output_path_data, output_path_length) ||
        !file_flush(output_path_data, output_path_length))
        goto table_csv_file_done;
    success = 1;
table_csv_file_done:
    if (!success) (void)file_delete(temporary_path_data, temporary_path_length);
    return success;
}

/* Export a bounded list to flushed temporary JSON and atomically promote it. */
int app_list_export_json_file_durable(int list_id,
    const char *output_path_data, unsigned __int64 output_path_length,
    const char *temporary_path_data, unsigned __int64 temporary_path_length) {
    static unsigned char document[JADREN_APP_LIST_DOCUMENT_MAX];
    unsigned __int64 json_length_slot = 0;
    int success = 0;
    if (!app_list_valid(list_id) || output_path_data == 0 || temporary_path_data == 0 ||
        output_path_length == 0 || temporary_path_length == 0 ||
        app_data_journal_export_path_same(output_path_data, output_path_length,
                                          temporary_path_data, temporary_path_length))
        return 0;
    if (file_exists(temporary_path_data, temporary_path_length) &&
        !file_delete(temporary_path_data, temporary_path_length))
        goto list_json_file_done;
    if (!app_list_export_json_exact(list_id, document, sizeof(document),
                                    &json_length_slot, 1))
        goto list_json_file_done;
    if (file_write(temporary_path_data, temporary_path_length,
                   document, json_length_slot) != json_length_slot)
        goto list_json_file_done;
    if (!file_flush(temporary_path_data, temporary_path_length) ||
        !file_replace_atomic(temporary_path_data, temporary_path_length,
                             output_path_data, output_path_length) ||
        !file_flush(output_path_data, output_path_length))
        goto list_json_file_done;
    success = 1;
list_json_file_done:
    if (!success) (void)file_delete(temporary_path_data, temporary_path_length);
    return success;
}

/* Export a bounded table to flushed temporary JSON and atomically promote it. */
int app_table_export_json_file_durable(int table_id,
    const char *output_path_data, unsigned __int64 output_path_length,
    const char *temporary_path_data, unsigned __int64 temporary_path_length) {
    static unsigned char document[JADREN_APP_TABLE_DOCUMENT_MAX];
    unsigned __int64 json_length_slot = 0;
    int success = 0;
    if (!app_table_valid(table_id) || output_path_data == 0 || temporary_path_data == 0 ||
        output_path_length == 0 || temporary_path_length == 0 ||
        app_data_journal_export_path_same(output_path_data, output_path_length,
                                          temporary_path_data, temporary_path_length))
        return 0;
    if (file_exists(temporary_path_data, temporary_path_length) &&
        !file_delete(temporary_path_data, temporary_path_length))
        goto table_json_file_done;
    if (!app_table_export_json_exact(table_id, document, sizeof(document),
                                     &json_length_slot, 1))
        goto table_json_file_done;
    if (file_write(temporary_path_data, temporary_path_length,
                   document, json_length_slot) != json_length_slot)
        goto table_json_file_done;
    if (!file_flush(temporary_path_data, temporary_path_length) ||
        !file_replace_atomic(temporary_path_data, temporary_path_length,
                             output_path_data, output_path_length) ||
        !file_flush(output_path_data, output_path_length))
        goto table_json_file_done;
    success = 1;
table_json_file_done:
    if (!success) (void)file_delete(temporary_path_data, temporary_path_length);
    return success;
}

/* Revision-guarded durable exports reject a stale caller snapshot before any
 * temporary or target file is touched. Cross-thread serialization remains
 * caller-owned, matching the other process-local guarded file APIs. */
int app_list_export_csv_file_durable_if_revision(
    int list_id,
    const char *output_path_data, unsigned __int64 output_path_length,
    const char *temporary_path_data, unsigned __int64 temporary_path_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_export_csv_file_durable(
        list_id, output_path_data, output_path_length,
        temporary_path_data, temporary_path_length);
}

int app_list_export_json_file_durable_if_revision(
    int list_id,
    const char *output_path_data, unsigned __int64 output_path_length,
    const char *temporary_path_data, unsigned __int64 temporary_path_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_list_export_json_file_durable(
        list_id, output_path_data, output_path_length,
        temporary_path_data, temporary_path_length);
}

int app_table_export_csv_file_durable_if_revision(
    int table_id,
    const char *output_path_data, unsigned __int64 output_path_length,
    const char *temporary_path_data, unsigned __int64 temporary_path_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_export_csv_file_durable(
        table_id, output_path_data, output_path_length,
        temporary_path_data, temporary_path_length);
}

int app_table_export_json_file_durable_if_revision(
    int table_id,
    const char *output_path_data, unsigned __int64 output_path_length,
    const char *temporary_path_data, unsigned __int64 temporary_path_length,
    unsigned __int64 expected_revision) {
    if (app_data_revision() != expected_revision) return 0;
    return app_table_export_json_file_durable(
        table_id, output_path_data, output_path_length,
        temporary_path_data, temporary_path_length);
}

/* Return one bounded page of indexed spans without copying journal payloads. */
unsigned __int64 app_data_journal_index_range_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *index_path, unsigned __int64 index_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 start_frame, unsigned __int64 max_frames,
    unsigned __int64 *output_data, unsigned __int64 output_length) {
    static unsigned char journal[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    static unsigned char index_data[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    static const char magic[] = "JADREN-APP-JINDEX-0.1\n";
    unsigned __int64 lock_token;
    unsigned __int64 journal_length;
    unsigned __int64 index_length;
    unsigned __int64 header_length = sizeof(magic) - 1 + 24;
    unsigned __int64 indexed_size;
    unsigned __int64 indexed_checksum;
    unsigned __int64 frame_count;
    unsigned __int64 page_count;
    unsigned __int64 page_end;
    unsigned __int64 entry_offset;
    unsigned __int64 previous_offset = 0;
    unsigned __int64 entry_index;
    unsigned __int64 output_index = 0;
    if (journal_path == 0 || index_path == 0 || lock_path == 0 || output_data == 0 ||
        journal_path_length == 0 || index_path_length == 0 || lock_path_length == 0 ||
        max_frames == 0 || max_frames > ((unsigned __int64)-1) / 3 ||
        output_length < 3) return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    journal_length = file_size(journal_path, journal_path_length);
    index_length = file_size(index_path, index_path_length);
    if (journal_length == 0 || journal_length > sizeof(journal) ||
        index_length < header_length || index_length > sizeof(index_data) ||
        file_read(journal_path, journal_path_length, journal, journal_length) != journal_length ||
        file_read(index_path, index_path_length, index_data, index_length) != index_length)
        goto index_range_done;
    for (entry_index = 0; entry_index < sizeof(magic) - 1; entry_index += 1)
        if (index_data[entry_index] != (unsigned char)magic[entry_index]) goto index_range_done;
    indexed_size = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1);
    indexed_checksum = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1 + 8);
    frame_count = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1 + 16);
    if (indexed_size != journal_length ||
        indexed_checksum != app_data_journal_checksum(journal, journal_length) ||
        frame_count > (index_length - header_length) / 24 ||
        header_length + frame_count * 24 != index_length ||
        start_frame >= frame_count)
        goto index_range_done;
    page_count = frame_count - start_frame;
    if (page_count > max_frames) page_count = max_frames;
    if (page_count > output_length / 3) goto index_range_done;
    page_end = start_frame + page_count;
    entry_offset = header_length;
    for (entry_index = 0; entry_index < frame_count; entry_index += 1) {
        unsigned __int64 current_offset = app_data_journal_index_get_u64(index_data, entry_offset);
        unsigned __int64 current_total = app_data_journal_index_get_u64(index_data, entry_offset + 8);
        unsigned __int64 current_payload = app_data_journal_index_get_u64(index_data, entry_offset + 16);
        if (current_payload == 0 || current_total <= current_payload ||
            current_offset > journal_length || current_total > journal_length - current_offset ||
            (entry_index > 0 && current_offset <= previous_offset))
            goto index_range_done;
        previous_offset = current_offset;
        entry_offset += 24;
    }
    if (!file_unlock(lock_token)) return 0;
    entry_offset = header_length;
    for (entry_index = 0; entry_index < frame_count; entry_index += 1) {
        if (entry_index >= start_frame && entry_index < page_end) {
            output_data[output_index++] = app_data_journal_index_get_u64(index_data, entry_offset);
            output_data[output_index++] = app_data_journal_index_get_u64(index_data, entry_offset + 8);
            output_data[output_index++] = app_data_journal_index_get_u64(index_data, entry_offset + 16);
        }
        entry_offset += 24;
    }
    return page_count;
index_range_done:
    (void)file_unlock(lock_token);
    return 0;
}

/* Read one bounded page of indexed frame payloads under one snapshot lock.
 * metadata is [journal_offset, total_bytes, payload_bytes, output_offset]
 * for each returned frame; no output is touched until every span and the
 * complete payload capacity have been validated. */
unsigned __int64 app_data_journal_index_read_page_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *index_path, unsigned __int64 index_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 start_frame, unsigned __int64 max_frames,
    unsigned char *output_data, unsigned __int64 output_length,
    unsigned __int64 *metadata_data, unsigned __int64 metadata_length) {
    static unsigned char journal[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    static unsigned char index_data[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    static const char magic[] = "JADREN-APP-JINDEX-0.1\n";
    unsigned __int64 lock_token;
    unsigned __int64 journal_length;
    unsigned __int64 index_length;
    unsigned __int64 header_length = sizeof(magic) - 1 + 24;
    unsigned __int64 indexed_size;
    unsigned __int64 indexed_checksum;
    unsigned __int64 frame_count;
    unsigned __int64 page_count;
    unsigned __int64 page_end;
    unsigned __int64 entry_offset;
    unsigned __int64 previous_offset = 0;
    unsigned __int64 entry_index;
    unsigned __int64 required_payload = 0;
    unsigned __int64 output_index = 0;
    unsigned __int64 metadata_index = 0;
    if (journal_path == 0 || index_path == 0 || lock_path == 0 ||
        output_data == 0 || metadata_data == 0 ||
        journal_path_length == 0 || index_path_length == 0 ||
        lock_path_length == 0 || max_frames == 0 ||
        max_frames > ((unsigned __int64)-1) / 4 || metadata_length < 4)
        return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    journal_length = file_size(journal_path, journal_path_length);
    index_length = file_size(index_path, index_path_length);
    if (journal_length == 0 || journal_length > sizeof(journal) ||
        index_length < header_length || index_length > sizeof(index_data) ||
        file_read(journal_path, journal_path_length, journal, journal_length) != journal_length ||
        file_read(index_path, index_path_length, index_data, index_length) != index_length)
        goto index_page_read_done;
    for (entry_index = 0; entry_index < sizeof(magic) - 1; entry_index += 1)
        if (index_data[entry_index] != (unsigned char)magic[entry_index])
            goto index_page_read_done;
    indexed_size = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1);
    indexed_checksum = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1 + 8);
    frame_count = app_data_journal_index_get_u64(index_data, sizeof(magic) - 1 + 16);
    if (indexed_size != journal_length ||
        indexed_checksum != app_data_journal_checksum(journal, journal_length) ||
        frame_count > (index_length - header_length) / 24 ||
        header_length + frame_count * 24 != index_length ||
        start_frame >= frame_count)
        goto index_page_read_done;
    page_count = frame_count - start_frame;
    if (page_count > max_frames) page_count = max_frames;
    if (page_count > (((unsigned __int64)-1) / 4) ||
        page_count * 4 > metadata_length)
        goto index_page_read_done;
    page_end = start_frame + page_count;
    entry_offset = header_length;
    for (entry_index = 0; entry_index < frame_count; entry_index += 1) {
        unsigned __int64 current_offset = app_data_journal_index_get_u64(index_data, entry_offset);
        unsigned __int64 current_total = app_data_journal_index_get_u64(index_data, entry_offset + 8);
        unsigned __int64 current_payload = app_data_journal_index_get_u64(index_data, entry_offset + 16);
        unsigned __int64 payload_start;
        if (current_payload == 0 || current_total <= current_payload ||
            current_offset > journal_length || current_total > journal_length - current_offset ||
            (entry_index > 0 && current_offset <= previous_offset))
            goto index_page_read_done;
        payload_start = current_offset + (current_total - current_payload);
        if (payload_start < current_offset || payload_start > journal_length ||
            current_payload > journal_length - payload_start)
            goto index_page_read_done;
        if (entry_index >= start_frame && entry_index < page_end) {
            if (current_payload > output_length - required_payload)
                goto index_page_read_done;
            required_payload += current_payload;
        }
        previous_offset = current_offset;
        entry_offset += 24;
    }
    entry_offset = header_length;
    for (entry_index = 0; entry_index < frame_count; entry_index += 1) {
        if (entry_index >= start_frame && entry_index < page_end) {
            unsigned __int64 current_offset = app_data_journal_index_get_u64(index_data, entry_offset);
            unsigned __int64 current_total = app_data_journal_index_get_u64(index_data, entry_offset + 8);
            unsigned __int64 current_payload = app_data_journal_index_get_u64(index_data, entry_offset + 16);
            unsigned __int64 payload_start = current_offset + (current_total - current_payload);
            unsigned __int64 digit;
            for (digit = 0; digit < current_payload; digit += 1)
                output_data[output_index + digit] = journal[payload_start + digit];
            metadata_data[metadata_index++] = current_offset;
            metadata_data[metadata_index++] = current_total;
            metadata_data[metadata_index++] = current_payload;
            metadata_data[metadata_index++] = output_index;
            output_index += current_payload;
        }
        entry_offset += 24;
    }
    if (!file_unlock(lock_token)) return 0;
    return page_count;
index_page_read_done:
    (void)file_unlock(lock_token);
    return 0;
}

/* Replay one zero-based complete frame into the caller-owned scratch path.
 * Invalid or torn tails are ignored, but the requested frame must itself pass
 * the v2 checksum when present. */
static int app_data_journal_recover_frame(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *scratch_path, unsigned __int64 scratch_path_length,
    unsigned __int64 frame_index) {
    static unsigned char journal[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    const char marker_v1[] = "JADREN-APP-JOURNAL-0.1\n";
    const char marker_v2[] = "JADREN-APP-JOURNAL-0.2\n";
    unsigned __int64 journal_length;
    unsigned __int64 index = 0;
    unsigned __int64 current_index = 0;
    if (journal_path == 0 || journal_path_length == 0 || scratch_path == 0 ||
        scratch_path_length == 0 ||
        (journal_length = file_size(journal_path, journal_path_length)) == 0 ||
        journal_length > sizeof(journal) ||
        file_read(journal_path, journal_path_length, journal, journal_length) != journal_length) {
        return 0;
    }
    while (index < journal_length) {
        unsigned __int64 parsed_length = 0;
        unsigned __int64 parsed_checksum = 0;
        unsigned __int64 digits_start;
        unsigned __int64 digit;
        unsigned __int64 marker_length = 0;
        int has_checksum = 0;
        if (sizeof(marker_v2) - 1 <= journal_length - index) {
            for (digit = 0; digit < sizeof(marker_v2) - 1; digit += 1)
                if (journal[index + digit] != (unsigned char)marker_v2[digit]) break;
            if (digit == sizeof(marker_v2) - 1) {
                marker_length = sizeof(marker_v2) - 1;
                has_checksum = 1;
            }
        }
        if (marker_length == 0 && sizeof(marker_v1) - 1 <= journal_length - index) {
            for (digit = 0; digit < sizeof(marker_v1) - 1; digit += 1)
                if (journal[index + digit] != (unsigned char)marker_v1[digit]) break;
            if (digit == sizeof(marker_v1) - 1) marker_length = sizeof(marker_v1) - 1;
        }
        if (marker_length == 0) break;
        index += marker_length;
        digits_start = index;
        while (index < journal_length && journal[index] != (unsigned char)'\n') {
            unsigned char value = journal[index];
            if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                parsed_length > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10)
                return 0;
            parsed_length = parsed_length * 10 + (value - (unsigned char)'0');
            index += 1;
        }
        if (index == digits_start || index >= journal_length || parsed_length == 0) break;
        index += 1;
        if (has_checksum) {
            digits_start = index;
            while (index < journal_length && journal[index] != (unsigned char)'\n') {
                unsigned char value = journal[index];
                if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                    parsed_checksum > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10)
                    return 0;
                parsed_checksum = parsed_checksum * 10 + (value - (unsigned char)'0');
                index += 1;
            }
            if (index == digits_start || index >= journal_length) break;
            index += 1;
        }
        if (parsed_length > journal_length - index ||
            (has_checksum && app_data_journal_checksum(journal + index, parsed_length) != parsed_checksum))
            break;
        if (current_index == frame_index) {
            if (file_write(scratch_path, scratch_path_length, journal + index, parsed_length) != parsed_length)
                return 0;
            return app_data_load(scratch_path, scratch_path_length);
        }
        current_index += 1;
        index += parsed_length;
    }
    return 0;
}

/* Preserve the last caller-selected number of complete valid frames.  The
 * operation rewrites only the journal bytes, so older valid checkpoints remain
 * available for explicit recovery while the lock covers the full replacement. */
int app_data_journal_retain_last_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *scratch_path, unsigned __int64 scratch_path_length,
    const char *temporary_path, unsigned __int64 temporary_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 max_frames) {
    static unsigned char journal[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    const char marker_v1[] = "JADREN-APP-JOURNAL-0.1\n";
    const char marker_v2[] = "JADREN-APP-JOURNAL-0.2\n";
    unsigned __int64 lock_token;
    unsigned __int64 frame_count;
    unsigned __int64 frames_to_skip;
    unsigned __int64 frame_index = 0;
    unsigned __int64 index = 0;
    unsigned __int64 retained_start = 0;
    unsigned __int64 retained_end = 0;
    unsigned __int64 journal_length;
    unsigned __int64 path_index;
    int compacted = 1;
    int temporary_is_journal;
    int temporary_is_scratch;
    (void)scratch_path;
    (void)scratch_path_length;
    if (journal_path == 0 || temporary_path == 0 || lock_path == 0 ||
        journal_path_length == 0 || temporary_path_length == 0 ||
        lock_path_length == 0 || max_frames == 0) return 0;
    temporary_is_journal = journal_path_length == temporary_path_length;
    temporary_is_scratch = scratch_path != 0 && scratch_path_length == temporary_path_length;
    if (temporary_is_journal) {
        for (path_index = 0; path_index < temporary_path_length; path_index += 1)
            if (journal_path[path_index] != temporary_path[path_index]) temporary_is_journal = 0;
    }
    if (temporary_is_scratch) {
        for (path_index = 0; path_index < temporary_path_length; path_index += 1)
            if (scratch_path[path_index] != temporary_path[path_index]) temporary_is_scratch = 0;
    }
    if (temporary_is_journal || temporary_is_scratch) return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    frame_count = app_data_journal_valid_frame_count(journal_path, journal_path_length);
    if (frame_count > max_frames) {
        frames_to_skip = frame_count - max_frames;
        journal_length = file_size(journal_path, journal_path_length);
        if (journal_length == 0 || journal_length > sizeof(journal) ||
            file_read(journal_path, journal_path_length, journal, journal_length) != journal_length) {
            compacted = 0;
        } else {
            while (index < journal_length) {
                unsigned __int64 frame_start = index;
                unsigned __int64 parsed_length = 0;
                unsigned __int64 parsed_checksum = 0;
                unsigned __int64 digits_start;
                unsigned __int64 digit;
                unsigned __int64 marker_length = 0;
                int has_checksum = 0;
                if (sizeof(marker_v2) - 1 <= journal_length - index) {
                    for (digit = 0; digit < sizeof(marker_v2) - 1; digit += 1)
                        if (journal[index + digit] != (unsigned char)marker_v2[digit]) break;
                    if (digit == sizeof(marker_v2) - 1) {
                        marker_length = sizeof(marker_v2) - 1;
                        has_checksum = 1;
                    }
                }
                if (marker_length == 0 && sizeof(marker_v1) - 1 <= journal_length - index) {
                    for (digit = 0; digit < sizeof(marker_v1) - 1; digit += 1)
                        if (journal[index + digit] != (unsigned char)marker_v1[digit]) break;
                    if (digit == sizeof(marker_v1) - 1) marker_length = sizeof(marker_v1) - 1;
                }
                if (marker_length == 0) break;
                index += marker_length;
                digits_start = index;
                while (index < journal_length && journal[index] != (unsigned char)'\n') {
                    unsigned char value = journal[index];
                    if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                        parsed_length > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10) {
                        index = journal_length;
                        break;
                    }
                    parsed_length = parsed_length * 10 + (value - (unsigned char)'0');
                    index += 1;
                }
                if (index == journal_length || index == digits_start || parsed_length == 0) break;
                index += 1;
                if (has_checksum) {
                    digits_start = index;
                    while (index < journal_length && journal[index] != (unsigned char)'\n') {
                        unsigned char value = journal[index];
                        if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                            parsed_checksum > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10) {
                            index = journal_length;
                            break;
                        }
                        parsed_checksum = parsed_checksum * 10 + (value - (unsigned char)'0');
                        index += 1;
                    }
                    if (index == journal_length || index == digits_start) break;
                    index += 1;
                }
                if (parsed_length > journal_length - index ||
                    (has_checksum && app_data_journal_checksum(journal + index, parsed_length) != parsed_checksum)) break;
                if (frame_index == frames_to_skip) retained_start = frame_start;
                frame_index += 1;
                index += parsed_length;
                retained_end = index;
            }
            if (frame_index != frame_count || retained_start >= retained_end ||
                file_write(temporary_path, temporary_path_length, journal + retained_start,
                           retained_end - retained_start) != retained_end - retained_start ||
                !file_flush(temporary_path, temporary_path_length) ||
                !file_replace_atomic(temporary_path, temporary_path_length,
                                     journal_path, journal_path_length) ||
                !file_flush(journal_path, journal_path_length) ||
                app_data_journal_valid_frame_count(journal_path, journal_path_length) > max_frames) {
                compacted = 0;
            }
        }
    }
    if (!file_unlock(lock_token)) return 0;
    return compacted;
}

/* Durable replay of one selected frame. The lock covers journal read, scratch
 * replacement and transactional model load. */
int app_data_journal_recover_frame_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *scratch_path, unsigned __int64 scratch_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 frame_index) {
    unsigned __int64 lock_token;
    int recovered;
    if (journal_path == 0 || scratch_path == 0 || lock_path == 0 ||
        journal_path_length == 0 || scratch_path_length == 0 || lock_path_length == 0)
        return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    recovered = app_data_journal_recover_frame(
        journal_path, journal_path_length, scratch_path, scratch_path_length, frame_index);
    if (!file_unlock(lock_token)) return 0;
    return recovered;
}

int app_data_journal_append(const char *journal_path,
                            unsigned __int64 journal_path_length,
                            const char *scratch_path,
                            unsigned __int64 scratch_path_length) {
    static unsigned char document[JADREN_APP_DATA_DOCUMENT_MAX];
    unsigned char length_data[32];
    unsigned char checksum_data[32];
    const char marker[] = "JADREN-APP-JOURNAL-0.2\n";
    unsigned __int64 document_length;
    unsigned __int64 length_length;
    unsigned __int64 checksum_length;
    unsigned __int64 checksum;
    if (journal_path == 0 || journal_path_length == 0 || scratch_path == 0 ||
        scratch_path_length == 0 || !app_data_save(scratch_path, scratch_path_length)) {
        return 0;
    }
    document_length = file_size(scratch_path, scratch_path_length);
    if (document_length == 0 || document_length > sizeof(document) ||
        file_read(scratch_path, scratch_path_length, document, document_length) != document_length) {
        return 0;
    }
    length_length = format_uint(document_length, length_data, sizeof(length_data));
    checksum = app_data_journal_checksum(document, document_length);
    checksum_length = format_uint(checksum, checksum_data, sizeof(checksum_data));
    if (length_length == 0 || checksum_length == 0 ||
        file_append_text(journal_path, journal_path_length, marker, sizeof(marker) - 1) != sizeof(marker) - 1 ||
        file_append(journal_path, journal_path_length, length_data, length_length, length_length) != length_length ||
        file_append_text(journal_path, journal_path_length, "\n", 1) != 1 ||
        file_append(journal_path, journal_path_length, checksum_data, checksum_length, checksum_length) != checksum_length ||
        file_append_text(journal_path, journal_path_length, "\n", 1) != 1 ||
        file_append(journal_path, journal_path_length, document, document_length, document_length) != document_length) {
        return 0;
    }
    return 1;
}

/* Durable journal append: acquire the caller-owned lock before serializing
 * the snapshot, append one complete frame, flush the journal and release the
 * lock. The lock path must be separate from journal_path. */
int app_data_journal_append_durable(const char *journal_path,
                                    unsigned __int64 journal_path_length,
                                    const char *scratch_path,
                                    unsigned __int64 scratch_path_length,
                                    const char *lock_path,
                                    unsigned __int64 lock_path_length) {
    unsigned __int64 lock_token;
    int appended;
    if (journal_path == 0 || journal_path_length == 0 || scratch_path == 0 ||
        scratch_path_length == 0 || lock_path == 0 || lock_path_length == 0) return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    appended = app_data_journal_append(journal_path, journal_path_length,
                                       scratch_path, scratch_path_length);
    if (appended && !file_flush(journal_path, journal_path_length)) appended = 0;
    if (!file_unlock(lock_token)) return 0;
    return appended;
}

int app_data_journal_recover(const char *journal_path,
                             unsigned __int64 journal_path_length,
                             const char *scratch_path,
                             unsigned __int64 scratch_path_length) {
    static unsigned char journal[JADREN_APP_JOURNAL_DOCUMENT_MAX];
    const char marker_v1[] = "JADREN-APP-JOURNAL-0.1\n";
    const char marker_v2[] = "JADREN-APP-JOURNAL-0.2\n";
    const unsigned char *latest_payload = 0;
    unsigned __int64 journal_length;
    unsigned __int64 index = 0;
    unsigned __int64 latest_length = 0;
    int have_frame = 0;
    if (journal_path == 0 || journal_path_length == 0 || scratch_path == 0 ||
        scratch_path_length == 0 ||
        (journal_length = file_size(journal_path, journal_path_length)) == 0 ||
        journal_length > sizeof(journal) ||
        file_read(journal_path, journal_path_length, journal, journal_length) != journal_length) {
        return 0;
    }
    while (index < journal_length) {
        unsigned __int64 parsed_length = 0;
        unsigned __int64 parsed_checksum = 0;
        unsigned __int64 digits_start;
        unsigned __int64 digit;
        unsigned __int64 marker_length = 0;
        int has_checksum = 0;
        if (sizeof(marker_v2) - 1 <= journal_length - index) {
            for (digit = 0; digit < sizeof(marker_v2) - 1; digit += 1)
                if (journal[index + digit] != (unsigned char)marker_v2[digit]) break;
            if (digit == sizeof(marker_v2) - 1) {
                marker_length = sizeof(marker_v2) - 1;
                has_checksum = 1;
            }
        }
        if (marker_length == 0 && sizeof(marker_v1) - 1 <= journal_length - index) {
            for (digit = 0; digit < sizeof(marker_v1) - 1; digit += 1)
                if (journal[index + digit] != (unsigned char)marker_v1[digit]) break;
            if (digit == sizeof(marker_v1) - 1) marker_length = sizeof(marker_v1) - 1;
        }
        if (marker_length == 0) break;
        index += marker_length;
        digits_start = index;
        while (index < journal_length && journal[index] != (unsigned char)'\n') {
            unsigned char value = journal[index];
            if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                parsed_length > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10) {
                return have_frame && file_write(scratch_path, scratch_path_length,
                                                latest_payload, latest_length) == latest_length &&
                       app_data_load(scratch_path, scratch_path_length);
            }
            parsed_length = parsed_length * 10 + (value - (unsigned char)'0');
            index += 1;
        }
        if (index == digits_start || index >= journal_length ||
            parsed_length == 0 || parsed_length > journal_length - index - 1) break;
        index += 1;
        if (has_checksum) {
            digits_start = index;
            while (index < journal_length && journal[index] != (unsigned char)'\n') {
                unsigned char value = journal[index];
                if (value < (unsigned char)'0' || value > (unsigned char)'9' ||
                    parsed_checksum > (((unsigned __int64)-1) - (value - (unsigned char)'0')) / 10) {
                    index = journal_length;
                    break;
                }
                parsed_checksum = parsed_checksum * 10 + (value - (unsigned char)'0');
                index += 1;
            }
            if (index == journal_length || index == digits_start || index >= journal_length - 1) break;
            index += 1;
            if (parsed_length > journal_length - index) break;
        }
        if (has_checksum && app_data_journal_checksum(journal + index, parsed_length) != parsed_checksum) break;
        latest_payload = journal + index;
        latest_length = parsed_length;
        have_frame = 1;
        index += parsed_length;
    }
    if (!have_frame || latest_payload == 0 ||
        file_write(scratch_path, scratch_path_length, latest_payload, latest_length) != latest_length) {
        return 0;
    }
    return app_data_load(scratch_path, scratch_path_length);
}

int app_data_journal_recover_compact(const char *journal_path,
                                     unsigned __int64 journal_path_length,
                                     const char *scratch_path,
                                     unsigned __int64 scratch_path_length,
                                     const char *temporary_path,
                                     unsigned __int64 temporary_path_length) {
    unsigned __int64 path_index;
    int temporary_is_journal = journal_path != 0 && temporary_path != 0 &&
                               journal_path_length == temporary_path_length;
    int temporary_is_scratch = scratch_path != 0 && temporary_path != 0 &&
                               scratch_path_length == temporary_path_length;
    int file_delete(const char *path_data, unsigned __int64 path_length);
    if (temporary_is_journal || temporary_is_scratch) {
        if (temporary_is_journal) {
            for (path_index = 0; path_index < temporary_path_length; path_index += 1)
                if (journal_path[path_index] != temporary_path[path_index]) temporary_is_journal = 0;
        }
        if (temporary_is_scratch) {
            for (path_index = 0; path_index < temporary_path_length; path_index += 1)
                if (scratch_path[path_index] != temporary_path[path_index]) temporary_is_scratch = 0;
        }
    }
    /* Recover first, then replace the journal with one canonical frame. */
    if (journal_path == 0 || scratch_path == 0 || temporary_path == 0 ||
        journal_path_length == 0 || scratch_path_length == 0 || temporary_path_length == 0 ||
        temporary_is_journal || temporary_is_scratch ||
        !app_data_journal_recover(journal_path, journal_path_length,
                                  scratch_path, scratch_path_length) ||
        (file_delete(temporary_path, temporary_path_length) == 0 &&
         file_size(temporary_path, temporary_path_length) != 0) ||
         !app_data_journal_append(temporary_path, temporary_path_length,
                                  scratch_path, scratch_path_length) ||
         !file_flush(temporary_path, temporary_path_length)) {
        return 0;
    }
    if (!file_replace_atomic(temporary_path, temporary_path_length,
                             journal_path, journal_path_length)) {
        return 0;
    }
    return file_flush(journal_path, journal_path_length);
}

/* Durable journal compaction: recover and rewrite one canonical frame while
 * holding the caller-owned lock for the whole operation. */
int app_data_journal_recover_compact_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *scratch_path, unsigned __int64 scratch_path_length,
    const char *temporary_path, unsigned __int64 temporary_path_length,
    const char *lock_path, unsigned __int64 lock_path_length) {
    unsigned __int64 lock_token;
    int compacted;
    if (journal_path == 0 || scratch_path == 0 || temporary_path == 0 ||
        lock_path == 0 || journal_path_length == 0 || scratch_path_length == 0 ||
        temporary_path_length == 0 || lock_path_length == 0) return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    compacted = app_data_journal_recover_compact(
        journal_path, journal_path_length, scratch_path, scratch_path_length,
        temporary_path, temporary_path_length);
    if (!file_unlock(lock_token)) return 0;
    return compacted;
}

/* Size-triggered durable rotation. The caller chooses the threshold; no
 * background worker or implicit retry is introduced. A journal above the
 * threshold is compacted to its newest valid frame while the lock is held. */
int app_data_journal_compact_if_over_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *scratch_path, unsigned __int64 scratch_path_length,
    const char *temporary_path, unsigned __int64 temporary_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 max_bytes) {
    unsigned __int64 lock_token;
    unsigned __int64 journal_size;
    int compacted = 1;
    if (journal_path == 0 || scratch_path == 0 || temporary_path == 0 ||
        lock_path == 0 || journal_path_length == 0 || scratch_path_length == 0 ||
        temporary_path_length == 0 || lock_path_length == 0 || max_bytes == 0) {
        return 0;
    }
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    journal_size = file_size(journal_path, journal_path_length);
    if (journal_size > max_bytes) {
        compacted = app_data_journal_recover_compact(
            journal_path, journal_path_length, scratch_path, scratch_path_length,
            temporary_path, temporary_path_length);
        if (compacted && file_size(journal_path, journal_path_length) > max_bytes) {
            compacted = 0;
        }
    }
    if (!file_unlock(lock_token)) return 0;
    return compacted;
}

/* Combined caller-driven maintenance policy. The lock covers the threshold
 * snapshot and the optional canonical compaction, so a scheduler tick never
 * races a concurrent append or retention pass. A missing journal is a
 * successful no-op; a nonzero limit is required for both dimensions. */
int app_data_journal_compact_if_needed_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *scratch_path, unsigned __int64 scratch_path_length,
    const char *temporary_path, unsigned __int64 temporary_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 max_bytes, unsigned __int64 max_frames) {
    unsigned __int64 lock_token;
    unsigned __int64 journal_size;
    unsigned __int64 frame_count;
    int compacted = 1;
    if (journal_path == 0 || scratch_path == 0 || temporary_path == 0 ||
        lock_path == 0 || journal_path_length == 0 || scratch_path_length == 0 ||
        temporary_path_length == 0 || lock_path_length == 0 || max_bytes == 0 ||
        max_frames == 0) return 0;
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    journal_size = file_size(journal_path, journal_path_length);
    frame_count = app_data_journal_valid_frame_count(journal_path, journal_path_length);
    if (journal_size > max_bytes || frame_count > max_frames) {
        compacted = app_data_journal_recover_compact(
            journal_path, journal_path_length, scratch_path, scratch_path_length,
            temporary_path, temporary_path_length);
        if (compacted && (file_size(journal_path, journal_path_length) > max_bytes ||
                          app_data_journal_valid_frame_count(journal_path, journal_path_length) > max_frames))
            compacted = 0;
    }
    if (!file_unlock(lock_token)) return 0;
    return compacted;
}

/* Bounded caller-driven retry around the lock-held maintenance boundary.
 * The runtime never creates a worker or an unbounded loop: max_attempts is
 * the total number of complete attempts and retry_delay_ms is an optional
 * sleep between failed attempts. */
static void app_data_journal_retry_sleep(unsigned __int64 retry_delay_ms) {
    if (retry_delay_ms == 0) return;
    Sleep(retry_delay_ms > 0xFFFFFFFFULL ? 0xFFFFFFFFUL :
          (unsigned long)retry_delay_ms);
}

int app_data_journal_maintenance_retry_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *scratch_path, unsigned __int64 scratch_path_length,
    const char *temporary_path, unsigned __int64 temporary_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 max_bytes, unsigned __int64 max_frames,
    unsigned __int64 max_attempts, unsigned __int64 retry_delay_ms) {
    unsigned __int64 attempt;
    if (journal_path == 0 || scratch_path == 0 || temporary_path == 0 ||
        lock_path == 0 || journal_path_length == 0 || scratch_path_length == 0 ||
        temporary_path_length == 0 || lock_path_length == 0 || max_bytes == 0 ||
        max_frames == 0 || max_attempts == 0) return 0;
    for (attempt = 0; attempt < max_attempts; attempt += 1) {
        if (app_data_journal_compact_if_needed_durable(
                journal_path, journal_path_length, scratch_path, scratch_path_length,
                temporary_path, temporary_path_length, lock_path, lock_path_length,
                max_bytes, max_frames)) return 1;
        if (attempt + 1 < max_attempts) app_data_journal_retry_sleep(retry_delay_ms);
    }
    return 0;
}

/* Frame-count-triggered durable rotation.  The caller chooses the threshold;
 * a journal above it is reduced to its newest valid frame while the lock is
 * held for recovery, flush and atomic replacement. */
int app_data_journal_compact_if_frames_over_durable(
    const char *journal_path, unsigned __int64 journal_path_length,
    const char *scratch_path, unsigned __int64 scratch_path_length,
    const char *temporary_path, unsigned __int64 temporary_path_length,
    const char *lock_path, unsigned __int64 lock_path_length,
    unsigned __int64 max_frames) {
    unsigned __int64 lock_token;
    unsigned __int64 frame_count;
    int compacted = 1;
    if (journal_path == 0 || scratch_path == 0 || temporary_path == 0 ||
        lock_path == 0 || journal_path_length == 0 || scratch_path_length == 0 ||
        temporary_path_length == 0 || lock_path_length == 0 || max_frames == 0) {
        return 0;
    }
    lock_token = file_lock(lock_path, lock_path_length);
    if (lock_token == 0) return 0;
    frame_count = app_data_journal_valid_frame_count(journal_path, journal_path_length);
    if (frame_count > max_frames) {
        compacted = app_data_journal_recover_compact(
            journal_path, journal_path_length, scratch_path, scratch_path_length,
            temporary_path, temporary_path_length);
        if (compacted && app_data_journal_valid_frame_count(journal_path, journal_path_length) > max_frames)
            compacted = 0;
    }
    if (!file_unlock(lock_token)) return 0;
    return compacted;
}

unsigned __int64 json_array_int(const long long *values_data,
                                unsigned __int64 values_length,
                                unsigned char *output_data,
                                unsigned __int64 output_length) {
    unsigned __int64 required = 2ULL;
    unsigned __int64 value_length;
    unsigned __int64 index;
    unsigned __int64 offset;
    unsigned char value_data[20];
    if (values_data == 0 && values_length > 0) {
        return 0;
    }
    for (index = 0; index < values_length; index += 1) {
        value_length = format_int(values_data[index], value_data, sizeof(value_data));
        if (value_length == 0 || required > ((unsigned __int64)-1) - value_length -
                                      (index == 0 ? 0ULL : 1ULL)) {
            return 0;
        }
        required += value_length + (index == 0 ? 0ULL : 1ULL);
    }
    if (output_data == 0 || output_length < required) {
        return 0;
    }
    output_data[0] = (unsigned char)'[';
    offset = 1;
    for (index = 0; index < values_length; index += 1) {
        if (index > 0) {
            output_data[offset] = (unsigned char)',';
            offset += 1;
        }
        value_length = format_int(values_data[index], value_data, sizeof(value_data));
        for (unsigned __int64 digit = 0; digit < value_length; digit += 1) {
            output_data[offset + digit] = value_data[digit];
        }
        offset += value_length;
    }
    output_data[offset] = (unsigned char)']';
    return required;
}

unsigned __int64 json_array_uint(const unsigned long long *values_data,
                                 unsigned __int64 values_length,
                                 unsigned char *output_data,
                                 unsigned __int64 output_length) {
    unsigned __int64 required = 2ULL;
    unsigned __int64 value_length;
    unsigned __int64 index;
    unsigned __int64 offset;
    unsigned char value_data[20];
    if (values_data == 0 && values_length > 0) {
        return 0;
    }
    for (index = 0; index < values_length; index += 1) {
        value_length = format_uint(values_data[index], value_data, sizeof(value_data));
        if (value_length == 0 || required > ((unsigned __int64)-1) - value_length -
                                      (index == 0 ? 0ULL : 1ULL)) {
            return 0;
        }
        required += value_length + (index == 0 ? 0ULL : 1ULL);
    }
    if (output_data == 0 || output_length < required) {
        return 0;
    }
    output_data[0] = (unsigned char)'[';
    offset = 1;
    for (index = 0; index < values_length; index += 1) {
        if (index > 0) {
            output_data[offset] = (unsigned char)',';
            offset += 1;
        }
        value_length = format_uint(values_data[index], value_data, sizeof(value_data));
        for (unsigned __int64 digit = 0; digit < value_length; digit += 1) {
            output_data[offset + digit] = value_data[digit];
        }
        offset += value_length;
    }
    output_data[offset] = (unsigned char)']';
    return required;
}

unsigned __int64 json_array_float(const double *values_data,
                                  unsigned __int64 values_length,
                                  unsigned char *output_data,
                                  unsigned __int64 output_length) {
    unsigned __int64 required = 2ULL;
    unsigned __int64 value_length;
    unsigned __int64 index;
    unsigned __int64 offset;
    unsigned char value_data[32];
    if (values_data == 0 && values_length > 0) {
        return 0;
    }
    for (index = 0; index < values_length; index += 1) {
        value_length = format_float(values_data[index], value_data, sizeof(value_data));
        if (value_length == 0 ||
            (value_length == 3 && (value_data[0] == (unsigned char)'n' ||
                                   value_data[0] == (unsigned char)'i')) ||
            (value_length == 4 && value_data[0] == (unsigned char)'-') ||
            required > ((unsigned __int64)-1) - value_length -
                           (index == 0 ? 0ULL : 1ULL)) {
            return 0;
        }
        required += value_length + (index == 0 ? 0ULL : 1ULL);
    }
    if (output_data == 0 || output_length < required) {
        return 0;
    }
    output_data[0] = (unsigned char)'[';
    offset = 1;
    for (index = 0; index < values_length; index += 1) {
        if (index > 0) {
            output_data[offset] = (unsigned char)',';
            offset += 1;
        }
        value_length = format_float(values_data[index], value_data, sizeof(value_data));
        for (unsigned __int64 digit = 0; digit < value_length; digit += 1) {
            output_data[offset + digit] = value_data[digit];
        }
        offset += value_length;
    }
    output_data[offset] = (unsigned char)']';
    return required;
}

unsigned __int64 json_array_bool(const unsigned char *values_data,
                                 unsigned __int64 values_length,
                                 unsigned char *output_data,
                                 unsigned __int64 output_length) {
    unsigned __int64 required = 2ULL;
    unsigned __int64 value_length;
    unsigned __int64 index;
    unsigned __int64 offset;
    unsigned char value_data[5];
    if (values_data == 0 && values_length > 0) {
        return 0;
    }
    for (index = 0; index < values_length; index += 1) {
        value_length = format_bool(values_data[index], value_data, sizeof(value_data));
        if (value_length == 0 || required > ((unsigned __int64)-1) - value_length -
                                      (index == 0 ? 0ULL : 1ULL)) {
            return 0;
        }
        required += value_length + (index == 0 ? 0ULL : 1ULL);
    }
    if (output_data == 0 || output_length < required) {
        return 0;
    }
    output_data[0] = (unsigned char)'[';
    offset = 1;
    for (index = 0; index < values_length; index += 1) {
        if (index > 0) {
            output_data[offset] = (unsigned char)',';
            offset += 1;
        }
        value_length = format_bool(values_data[index], value_data, sizeof(value_data));
        for (unsigned __int64 digit = 0; digit < value_length; digit += 1) {
            output_data[offset + digit] = value_data[digit];
        }
        offset += value_length;
    }
    output_data[offset] = (unsigned char)']';
    return required;
}

int file_delete(const char *path_data, unsigned __int64 path_length) {
    wchar_t wide_path[1024];
    return path_to_wide(path_data, path_length, wide_path, 1024) &&
           DeleteFileW(wide_path);
}

/* Deletes a file through an explicit valid caller-owned UTF-8 path. */
int file_delete_path(const unsigned char *path_data, unsigned __int64 path_capacity,
                     unsigned __int64 path_length) {
    if (path_length > path_capacity) return 0;
    return file_delete((const char *)path_data, path_length);
}

/* Non-blocking cross-process lock represented by the live OS handle. The
 * caller must keep the token and release it with file_unlock. */
unsigned __int64 file_lock(const char *path_data, unsigned __int64 path_length) {
    wchar_t wide_path[1024];
    HANDLE handle;
    if (!path_to_wide(path_data, path_length, wide_path, 1024)) {
        return 0;
    }
    handle = CreateFileW(wide_path, GENERIC_READ | GENERIC_WRITE, 0, 0,
                         OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    if (handle == INVALID_HANDLE_VALUE) {
        return 0;
    }
    return (unsigned __int64)handle;
}

/* Acquires a non-blocking lock through an explicit valid caller-owned path. */
unsigned __int64 file_lock_path(const unsigned char *path_data,
                                unsigned __int64 path_capacity,
                                unsigned __int64 path_length) {
    if (path_length > path_capacity) return 0;
    return file_lock((const char *)path_data, path_length);
}

/* Retries a caller-owned sidecar lock a finite caller-selected number of
 * times. Each failed attempt except the last sleeps for retry_delay_ms on the
 * calling thread; this is deliberately not an event-loop or real-time API. */
unsigned __int64 file_lock_path_retry(const unsigned char *path_data,
                                      unsigned __int64 path_capacity,
                                      unsigned __int64 path_length,
                                      unsigned __int64 max_attempts,
                                      unsigned __int64 retry_delay_ms) {
    unsigned __int64 attempt = 0;
    if (path_length > path_capacity || max_attempts == 0) return 0;
    while (attempt < max_attempts) {
        unsigned __int64 token = file_lock((const char *)path_data, path_length);
        if (token != 0) return token;
        attempt += 1;
        if (attempt < max_attempts && retry_delay_ms != 0) {
            Sleep(retry_delay_ms > 0xFFFFFFFFULL ? 0xFFFFFFFFUL :
                  (unsigned long)retry_delay_ms);
        }
    }
    return 0;
}

int file_unlock(unsigned __int64 token) {
    if (token == 0 || token == (unsigned __int64)-1) {
        return 0;
    }
    return CloseHandle((HANDLE)token);
}

int file_replace_atomic(const char *source_data, unsigned __int64 source_length,
                        const char *target_data, unsigned __int64 target_length) {
    wchar_t source_path[1024];
    wchar_t target_path[1024];
    if (!path_to_wide(source_data, source_length, source_path, 1024) ||
        !path_to_wide(target_data, target_length, target_path, 1024)) {
        return 0;
    }
    return MoveFileExW(source_path, target_path,
                       MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH);
}

/* Atomically promotes an explicit valid caller-owned source path over target. */
int file_replace_atomic_paths(const unsigned char *source_data,
                              unsigned __int64 source_capacity,
                              unsigned __int64 source_length,
                              const unsigned char *target_data,
                              unsigned __int64 target_capacity,
                              unsigned __int64 target_length) {
    if (source_length > source_capacity || target_length > target_capacity) return 0;
    return file_replace_atomic((const char *)source_data, source_length,
                               (const char *)target_data, target_length);
}

/* Writes a valid caller-owned prefix, flushes it, then atomically promotes it. */
int file_write_atomic(const char *temporary_path_data,
                      unsigned __int64 temporary_path_length,
                      const char *target_path_data,
                      unsigned __int64 target_path_length,
                      const unsigned char *input_data,
                      unsigned __int64 input_length,
                      unsigned __int64 write_length) {
    unsigned __int64 path_index;
    int same_path;
    if (temporary_path_data == 0 || target_path_data == 0 ||
        temporary_path_length == 0 || target_path_length == 0) return 0;
    same_path = temporary_path_length == target_path_length;
    if (same_path) {
        for (path_index = 0; path_index < temporary_path_length; path_index += 1) {
            if (temporary_path_data[path_index] != target_path_data[path_index]) {
                same_path = 0;
                break;
            }
        }
    }
    if (same_path ||
        write_length > input_length ||
        write_file_bytes(temporary_path_data, temporary_path_length,
                         input_data, write_length) != write_length) {
        return 0;
    }
    if (!file_flush(temporary_path_data, temporary_path_length)) return 0;
    return file_replace_atomic(temporary_path_data, temporary_path_length,
                               target_path_data, target_path_length);
}

/* Durable caller-thread commit including promoted-file and directory flush. */
int file_write_atomic_durable(const char *temporary_path_data,
                              unsigned __int64 temporary_path_length,
                              const char *target_path_data,
                              unsigned __int64 target_path_length,
                              const char *directory_path_data,
                              unsigned __int64 directory_path_length,
                              const unsigned char *input_data,
                              unsigned __int64 input_length,
                              unsigned __int64 write_length) {
    if (directory_path_data == 0 || directory_path_length == 0 ||
        !directory_exists(directory_path_data, directory_path_length) ||
        !file_write_atomic(temporary_path_data, temporary_path_length,
                           target_path_data, target_path_length, input_data,
                           input_length, write_length)) {
        return 0;
    }
    if (!file_flush(target_path_data, target_path_length)) return 0;
    return directory_flush(directory_path_data, directory_path_length);
}
