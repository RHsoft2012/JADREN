#ifndef JADREN_UI_HAS_FILE_RUNTIME
#define JADREN_UI_HAS_FILE_RUNTIME 0
#endif

/*
 * Minimal freestanding Win32 desktop runtime for the Jadren Windows preview.
 *
 * The public preview API is deliberately small: Jadren source first declares a
 * window and then its labels, status and buttons.  This native part only owns
 * the Win32 class, message loop and drawing; all visible text, geometry and
 * colours come from the Jadren program. UI 0.2 additionally allows an
 * explicitly exported Jadren callback for event buttons; the native host still
 * owns only the Win32 message boundary.
 *
 * This source avoids the CRT and Windows SDK headers.  The CLI compiles it with
 * the pinned LLVM toolchain and creates the explicit import libraries below.
 */

typedef unsigned int UINT;
typedef unsigned long DWORD;
typedef unsigned __int64 UINT_PTR;
typedef int BOOL;
typedef long LONG;
typedef long long LONGLONG;
typedef __int64 LONG_PTR;
typedef unsigned short wchar_t;
typedef unsigned __int64 WPARAM;
typedef __int64 LPARAM;
typedef __int64 LRESULT;
typedef void *HANDLE;
typedef HANDLE HWND;
typedef HANDLE HINSTANCE;
typedef HANDLE HICON;
typedef HANDLE HCURSOR;
typedef HANDLE HBRUSH;
typedef HANDLE HMENU;
typedef HANDLE HDC;
typedef HANDLE HRGN;
typedef HANDLE GpImage;
typedef HANDLE GpGraphics;
typedef const wchar_t *LPCWSTR;

typedef struct JadrenString {
    const char *data;
    LONGLONG length;
} JadrenString;

/* Jadren executables are freestanding. LLVM emits this Windows x64 helper
 * whenever one source function needs more than one stack page (for example a
 * substantial UI event handler). Normally it would come from the MSVC CRT;
 * provide the real probing routine here so preview programs stay CRT-free.
 * RAX is the requested allocation and must be preserved for the caller. */
__asm__(
    ".text\n"
    ".globl __chkstk\n"
    ".p2align 4, 0x90\n"
    "__chkstk:\n"
    "  pushq %rcx\n"
    "  pushq %rax\n"
    "  cmpq $4096, %rax\n"
    "  leaq 24(%rsp), %rcx\n"
    "  jb .Ljadren_chkstk_done\n"
    ".Ljadren_chkstk_loop:\n"
    "  subq $4096, %rcx\n"
    "  testq %rax, (%rcx)\n"
    "  subq $4096, %rax\n"
    "  cmpq $4096, %rax\n"
    "  jae .Ljadren_chkstk_loop\n"
    ".Ljadren_chkstk_done:\n"
    "  subq %rax, %rcx\n"
    "  testq %rax, (%rcx)\n"
    "  popq %rax\n"
    "  popq %rcx\n"
    "  retq\n");

typedef LRESULT (*WNDPROC)(HWND, UINT, WPARAM, LPARAM);

typedef struct POINT {
    LONG x;
    LONG y;
} POINT;

typedef struct RECT {
    LONG left;
    LONG top;
    LONG right;
    LONG bottom;
} RECT;

typedef struct MSG {
    HWND hwnd;
    UINT message;
    WPARAM wParam;
    LPARAM lParam;
    DWORD time;
    POINT pt;
    DWORD lPrivate;
} MSG;

typedef struct WNDCLASSEXW {
    UINT cbSize;
    UINT style;
    WNDPROC lpfnWndProc;
    int cbClsExtra;
    int cbWndExtra;
    HINSTANCE hInstance;
    HICON hIcon;
    HCURSOR hCursor;
    HBRUSH hbrBackground;
    LPCWSTR lpszMenuName;
    LPCWSTR lpszClassName;
    HICON hIconSm;
} WNDCLASSEXW;

typedef struct INITCOMMONCONTROLSEX {
    DWORD dwSize;
    DWORD dwICC;
} INITCOMMONCONTROLSEX;

typedef struct NMHDR {
    HWND hwndFrom;
    UINT_PTR idFrom;
    UINT code;
} NMHDR;

typedef struct NMLISTVIEW {
    NMHDR hdr;
    int iItem;
    int iSubItem;
    UINT uNewState;
    UINT uOldState;
    UINT uChanged;
    POINT ptAction;
    LPARAM lParam;
} NMLISTVIEW;

typedef struct LVCOLUMNW {
    UINT mask;
    int fmt;
    int cx;
    wchar_t *pszText;
    int cchTextMax;
    int iSubItem;
    int iImage;
    int iOrder;
    int cxMin;
    int cxDefault;
    int cxIdeal;
} LVCOLUMNW;

typedef struct LVITEMW {
    UINT mask;
    int iItem;
    int iSubItem;
    UINT state;
    UINT stateMask;
    wchar_t *pszText;
    int cchTextMax;
    int iImage;
    LPARAM lParam;
    int iIndent;
    int iGroupId;
    UINT cColumns;
    UINT *puColumns;
    int *piColFmt;
    int iGroup;
} LVITEMW;

typedef struct TOOLINFOW {
    UINT cbSize;
    UINT uFlags;
    HWND hwnd;
    UINT_PTR uId;
    RECT rect;
    HINSTANCE hinst;
    wchar_t *lpszText;
    LPARAM lParam;
    void *lpReserved;
} TOOLINFOW;

typedef struct DRAWITEMSTRUCT {
    UINT CtlType;
    UINT CtlID;
    UINT itemID;
    UINT itemAction;
    UINT itemState;
    HWND hwndItem;
    HDC hDC;
    RECT rcItem;
    HANDLE itemData;
} DRAWITEMSTRUCT;

typedef struct PAINTSTRUCT {
    HDC hdc;
    BOOL erase;
    RECT paint;
    BOOL restore;
    BOOL update;
    unsigned char reserved[32];
} PAINTSTRUCT;

typedef struct GdiplusStartupInput {
    UINT version;
    void *debug_event_callback;
    BOOL suppress_background_thread;
    BOOL suppress_external_codecs;
} GdiplusStartupInput;

typedef struct MINMAXINFO {
    POINT ptReserved;
    POINT ptMaxSize;
    POINT ptMaxPosition;
    POINT ptMinTrackSize;
    POINT ptMaxTrackSize;
} MINMAXINFO;

typedef struct TRACKMOUSEEVENT {
    DWORD cbSize;
    DWORD dwFlags;
    HWND hwndTrack;
    DWORD dwHoverTime;
} TRACKMOUSEEVENT;

typedef struct LayoutChild {
    int kind;
    int index;
    int width;
    int height;
    BOOL stretch;
} LayoutChild;

typedef struct LayoutNode {
    int parent;
    int orientation;
    int x;
    int y;
    int width;
    int height;
    int base_width;
    int base_height;
    int padding;
    int gap;
    int align;
    BOOL stretch;
    int panel_label;
    LayoutChild children[24];
    int child_count;
} LayoutNode;

typedef struct InputSpec {
    wchar_t text[384];
    int x;
    int y;
    int width;
    int height;
    UINT text_color;
    UINT background_color;
    int corner_radius;
    int event_id;
    UINT id;
    BOOL updating;
    BOOL layout_managed;
    WNDPROC default_proc;
    HWND handle;
    HBRUSH brush;
} InputSpec;

typedef struct SelectSpec {
    wchar_t options[12][160];
    int option_count;
    int selected_index;
    int x;
    int y;
    int width;
    int height;
    UINT text_color;
    UINT background_color;
    int corner_radius;
    int event_id;
    UINT id;
    BOOL layout_managed;
    HWND handle;
    HBRUSH brush;
} SelectSpec;

typedef struct ScrollSpec {
    wchar_t text[2048];
    int x;
    int y;
    int width;
    int height;
    UINT text_color;
    UINT background_color;
    int corner_radius;
    UINT id;
    HWND handle;
    HBRUSH brush;
} ScrollSpec;

typedef struct ListSpec {
    wchar_t items[32][160];
    int item_count;
    int selected_index;
    int x;
    int y;
    int width;
    int height;
    UINT text_color;
    UINT background_color;
    int corner_radius;
    int event_id;
    UINT id;
    int app_list_id;
    BOOL layout_managed;
    HWND handle;
    HBRUSH brush;
} ListSpec;

/* Bounded native report-view table. The model is deliberately fixed-size for
 * the preview runtime: Jadren owns the row/cell declarations and callbacks
 * mutate them explicitly, without hidden heap allocation in the UI thread. */
typedef struct TableSpec {
    wchar_t headers[8][160];
    int header_widths[8];
    int column_count;
    wchar_t cells[64][8][160];
    int row_count;
    int selected_row;
    int x;
    int y;
    int width;
    int height;
    UINT text_color;
    UINT background_color;
    int corner_radius;
    int event_id;
    UINT id;
    int app_table_id;
    int app_table_column_count;
    BOOL layout_managed;
    BOOL updating;
    HWND handle;
    HBRUSH brush;
} TableSpec;

/* A small explicit bridge between native control values and Jadren's
 * caller-owned integer state slots. Binding metadata is fixed-size and lives
 * with the desktop model; it never allocates or hides a callback. */
typedef struct StateBinding {
    int event_id;
    int slot;
    int mode;
} StateBinding;

typedef struct InputAppBinding {
    int event_id;
    char key[65];
    unsigned __int64 key_length;
    int kind;
} InputAppBinding;

enum {
    JADREN_APP_BIND_TEXT = 1,
    JADREN_APP_BIND_BOOL = 2,
    JADREN_APP_BIND_SELECT = 3,
    JADREN_APP_BIND_LIST = 4,
    JADREN_APP_BIND_TABLE = 5
};

typedef struct MenuSpec {
    wchar_t options[12][160];
    int option_event_ids[12];
    int option_count;
    int menu_id;
    UINT button_id;
} MenuSpec;

/* Image assets are deliberately painted by the parent behind normal controls,
 * so PNG artwork never creates a separate native child that can leave resize
 * ghosts. The first UI 0.2 asset path is raster PNG. */
typedef struct ImageSpec {
    wchar_t path[520];
    int x;
    int y;
    int width;
    int height;
    GpImage image;
} ImageSpec;

/* A tooltip is associated with a Jadren event id. Its handle is the native
 * Windows tooltip control, which owns timing, placement and compositing. */
typedef struct TooltipSpec {
    wchar_t text[384];
    int event_id;
    int width;
    int height;
    UINT text_color;
    UINT background_color;
    int corner_radius;
    HWND handle;
} TooltipSpec;

typedef struct LabelSpec {
    wchar_t text[384];
    int x;
    int y;
    int width;
    int height;
    UINT text_color;
    UINT background_color;
    int corner_radius;
    BOOL fills_width;
    BOOL layout_managed;
    HWND handle;
    HBRUSH brush;
} LabelSpec;

typedef struct ButtonSpec {
    wchar_t label[160];
    wchar_t action[384];
    int x;
    int y;
    int current_x;
    int current_y;
    int width;
    int height;
    UINT text_color;
    UINT background_color;
    int corner_radius;
    UINT id;
    BOOL closes_window;
    BOOL flat_style;
    BOOL right_anchored;
    BOOL distributes_horizontally;
    BOOL is_menu_item;
    BOOL toggles;
    BOOL active;
    BOOL disabled;
    BOOL system_icon;
    BOOL hovered;
    BOOL pressed;
    BOOL tracks_mouse;
    int event_id;
    BOOL emits_event;
    BOOL layout_managed;
    BOOL is_checkbox;
    BOOL is_switch;
    WNDPROC default_proc;
    HWND handle;
    HBRUSH brush;
    HANDLE font;
} ButtonSpec;

typedef struct DesktopSpec {
    wchar_t title[256];
    int width;
    int height;
    int min_width;
    int min_height;
    int max_width;
    int max_height;
    UINT background_color;
    int base_client_width;
    int base_client_height;
    HBRUSH background_brush;
    HBRUSH border_brush;
    HWND window;
    LabelSpec labels[32];
    int label_count;
    int status_index;
    ButtonSpec buttons[24];
    int button_count;
    InputSpec inputs[12];
    int input_count;
    SelectSpec selects[8];
    int select_count;
    ScrollSpec scrolls[8];
    int scroll_count;
    ListSpec lists[8];
    int list_count;
    TableSpec tables[4];
    int table_count;
    InputAppBinding input_app_bindings[12];
    int input_app_binding_count;
    MenuSpec menus[8];
    int menu_count;
    ImageSpec images[12];
    int image_count;
    TooltipSpec tooltips[16];
    int tooltip_count;
    int state_slots[32];
    wchar_t state_text[32][256];
    StateBinding state_bindings[64];
    int state_binding_count;
    LayoutNode layouts[16];
    int layout_count;
    int layout_stack[16];
    int layout_depth;
    /* Child creation can emit WM_COMMAND. Do not call Jadren before the
     * complete control tree exists and its source function has returned. */
    BOOL events_ready;
    int resize_event_id;
    int close_event_id;
    BOOL close_event_sent;
} DesktopSpec;

extern HINSTANCE GetModuleHandleW(LPCWSTR name);
extern DWORD GetModuleFileNameW(HINSTANCE instance, wchar_t *filename, DWORD size);
extern void ExitProcess(UINT result);

/* Keep Float64 desktop programs CRT-free. File-enabled desktop programs
 * receive the same marker from the file runtime, so avoid a duplicate symbol. */
#if !JADREN_UI_HAS_FILE_RUNTIME
int _fltused = 0;
#endif

/* Keep the freestanding desktop target compatible with Jadren's checked
 * array/slice lowering. The compiler emits this ABI symbol for bounds checks;
 * the preview runtime has no CRT, so the process boundary is the panic sink.
 * File-enabled desktop programs already link the same symbol from the file
 * runtime, hence the conditional definition. */
#if !JADREN_UI_HAS_FILE_RUNTIME
void jadren_rt_bounds_panic_u64(unsigned __int64 index, unsigned __int64 length) {
    (void)index;
    (void)length;
    ExitProcess(1);
}
#endif

#if JADREN_UI_HAS_FILE_RUNTIME
extern int app_list_count(int list_id);
extern unsigned __int64 app_list_read_text(int list_id, int item_index,
                                           unsigned char *output_data,
                                           unsigned __int64 output_length);
extern int app_table_row_count(int table_id);
extern unsigned __int64 app_table_read_cell(int table_id, int row_index,
                                            int column_index,
                                            unsigned char *output_data,
                                            unsigned __int64 output_length);
extern int app_table_sort_text(int table_id, int column_index, int descending);
extern int app_table_sort_int(int table_id, int column_index, int descending);
extern int app_table_sort_uint(int table_id, int column_index, int descending);
extern int app_table_sort_float(int table_id, int column_index, int descending);
extern int app_table_sort_bool(int table_id, int column_index, int descending);
extern int app_table_filter_text(int source_table_id, int destination_table_id,
                                 int column_index, const char *query_data,
                                 unsigned __int64 query_length);
extern int app_table_filter_text_ex(int source_table_id, int destination_table_id,
                                    int column_index, const char *query_data,
                                    unsigned __int64 query_length, int mode);
extern int app_table_filter_int(int source_table_id, int destination_table_id,
                                int column_index, long long query);
extern int app_table_filter_uint(int source_table_id, int destination_table_id,
                                 int column_index, unsigned __int64 query);
extern int app_table_filter_float(int source_table_id, int destination_table_id,
                                  int column_index, double query);
extern int app_table_filter_bool(int source_table_id, int destination_table_id,
                                 int column_index, unsigned char query);
extern int app_state_set_text(const char *key_data, unsigned __int64 key_length,
                              const char *value_data, unsigned __int64 value_length);
extern unsigned __int64 app_state_read_text(const char *key_data,
                                            unsigned __int64 key_length,
                                            unsigned char *output_data,
                                            unsigned __int64 output_length);
extern int app_state_set_bool(const char *key_data, unsigned __int64 key_length,
                              unsigned char value);
extern int app_state_get_bool(const char *key_data, unsigned __int64 key_length);
extern int app_state_set_int(const char *key_data, unsigned __int64 key_length,
                             long long value);
extern long long app_state_get_int(const char *key_data, unsigned __int64 key_length);
#endif

extern int MultiByteToWideChar(UINT code_page, DWORD flags, const char *source,
                               int source_length, wchar_t *target, int target_length);
extern int WideCharToMultiByte(UINT code_page, DWORD flags, const wchar_t *source,
                               int source_length, char *target, int target_length,
                               const char *default_character, BOOL *used_default_character);
extern HCURSOR LoadCursorW(HINSTANCE instance, LPCWSTR cursor_name);
extern HMENU CreatePopupMenu(void);
extern BOOL AppendMenuW(HMENU menu, UINT flags, UINT_PTR identifier, LPCWSTR text);
extern unsigned short RegisterClassExW(const WNDCLASSEXW *window_class);
extern HWND CreateWindowExW(DWORD extended_style, LPCWSTR class_name, LPCWSTR title,
                            DWORD style, int x, int y, int width, int height,
                            HWND parent, HMENU menu, HINSTANCE instance, void *parameter);
extern LRESULT CallWindowProcW(WNDPROC previous, HWND window, UINT message,
                               WPARAM wparam, LPARAM lparam);
extern LRESULT DefWindowProcW(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
extern BOOL DestroyWindow(HWND window);
extern BOOL DestroyMenu(HMENU menu);
extern LRESULT DispatchMessageW(const MSG *message);
extern int DrawTextW(HDC device_context, LPCWSTR text, int count, RECT *rectangle, UINT format);
extern BOOL EnableWindow(HWND window, BOOL enabled);
extern HWND GetDlgItem(HWND window, int id);
extern BOOL IsWindow(HWND window);
extern int FillRect(HDC device_context, const RECT *rectangle, HBRUSH brush);
extern int FrameRect(HDC device_context, const RECT *rectangle, HBRUSH brush);
extern BOOL GetCursorPos(POINT *point);
extern BOOL GetClientRect(HWND window, RECT *rectangle);
extern int GetWindowTextW(HWND window, wchar_t *text, int capacity);
extern int GetMessageW(MSG *message, HWND window, UINT minimum, UINT maximum);
extern int MessageBoxW(HWND window, LPCWSTR text, LPCWSTR title, UINT style);
extern BOOL MoveWindow(HWND window, int x, int y, int width, int height, BOOL repaint);
extern BOOL PostMessageW(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
extern void PostQuitMessage(int result);
extern BOOL RedrawWindow(HWND window, const RECT *update_rectangle, HRGN update_region,
                         UINT flags);
extern BOOL SetForegroundWindow(HWND window);
extern BOOL SetWindowPos(HWND window, HWND insert_after, int x, int y,
                         int width, int height, UINT flags);
extern BOOL SetWindowTextW(HWND window, LPCWSTR text);
extern LRESULT SendMessageW(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
extern int SetWindowRgn(HWND window, HRGN region, BOOL redraw);
extern LONG_PTR SetWindowLongPtrW(HWND window, int index, LONG_PTR value);
extern BOOL ShowWindow(HWND window, int command);
extern BOOL TranslateMessage(const MSG *message);
extern BOOL TrackMouseEvent(TRACKMOUSEEVENT *event);
extern UINT TrackPopupMenu(HMENU menu, UINT flags, int x, int y, int reserved,
                           HWND window, const RECT *excluded_rectangle);
extern BOOL UpdateWindow(HWND window);
extern HBRUSH CreateSolidBrush(DWORD color);
extern HANDLE CreateFontW(int height, int width, int escapement, int orientation,
                          int weight, DWORD italic, DWORD underline, DWORD strikeout,
                          DWORD charset, DWORD output_precision, DWORD clip_precision,
                          DWORD quality, DWORD pitch_and_family, LPCWSTR face_name);
extern HDC BeginPaint(HWND window, PAINTSTRUCT *paint);
extern BOOL EndPaint(HWND window, const PAINTSTRUCT *paint);
extern HRGN CreateRoundRectRgn(int left, int top, int right, int bottom,
                               int width, int height);
extern BOOL DeleteObject(HANDLE object);
extern int GdiplusStartup(UINT_PTR *token, const GdiplusStartupInput *input, void *output);
extern void GdiplusShutdown(UINT_PTR token);
extern int GdipCreateBitmapFromFile(LPCWSTR filename, GpImage *bitmap);
extern int GdipDisposeImage(GpImage image);
extern int GdipCreateFromHDC(HDC device_context, GpGraphics *graphics);
extern int GdipDeleteGraphics(GpGraphics graphics);
extern int GdipDrawImageRectI(GpGraphics graphics, GpImage image,
                              int x, int y, int width, int height);
extern BOOL FillRgn(HDC device_context, HRGN region, HBRUSH brush);
extern BOOL FrameRgn(HDC device_context, HRGN region, HBRUSH brush,
                     int width, int height);
extern DWORD SetBkColor(HDC device_context, DWORD color);
extern int SetBkMode(HDC device_context, int mode);
extern DWORD SetTextColor(HDC device_context, DWORD color);
extern BOOL InitCommonControlsEx(const INITCOMMONCONTROLSEX *controls);

#if JADREN_UI_HAS_EVENT_CALLBACK
extern int jadren_ui_on_click(int event_id);
#endif

/* Model bindings are refreshed at the end of every Jadren UI callback.  The
 * explicit API below is also available for code that mutates its model before
 * entering the message loop or from a non-event helper. */
static void refresh_app_bindings(void);
static void sync_app_bindings_from_native(void);

enum {
    CP_UTF8 = 65001,
    WM_COMMAND = 0x0111,
    WM_NOTIFY = 0x004E,
    WM_CANCELMODE = 0x001F,
    WM_CTLCOLORSTATIC = 0x0138,
    WM_CTLCOLOREDIT = 0x0133,
    WM_CTLCOLORLISTBOX = 0x0134,
    WM_ERASEBKGND = 0x0014,
    WM_DESTROY = 0x0002,
    WM_PAINT = 0x000F,
    WM_DRAWITEM = 0x002B,
    WM_GETMINMAXINFO = 0x0024,
    WM_LBUTTONDOWN = 0x0201,
    WM_LBUTTONUP = 0x0202,
    WM_MOUSELEAVE = 0x02A3,
    WM_MOUSEMOVE = 0x0200,
    WM_CAPTURECHANGED = 0x0215,
    WM_SETTEXT = 0x000C,
    WM_CHAR = 0x0102,
    WM_PASTE = 0x0302,
    WM_CUT = 0x0300,
    WM_CLEAR = 0x0303,
    WM_JADREN_SET_INPUT_TEXT = 0x8001,
    WM_SIZE = 0x0005,
    WS_CHILD = 0x40000000,
    WS_DISABLED = 0x08000000,
    WS_VISIBLE = 0x10000000,
    WS_TABSTOP = 0x00010000,
    WS_BORDER = 0x00800000,
    WS_VSCROLL = 0x00200000,
    WS_HSCROLL = 0x00100000,
    WS_OVERLAPPEDWINDOW = 0x00CF0000,
    WS_POPUP = 0x80000000,
    WS_EX_TOPMOST = 0x00000008,
    WS_EX_TOOLWINDOW = 0x00000080,
    WS_EX_NOACTIVATE = 0x08000000,
    BS_OWNERDRAW = 0x0000000B,
    BS_NOTIFY = 0x00004000,
    CS_HREDRAW = 0x0002,
    CS_VREDRAW = 0x0001,
    GWLP_WNDPROC = -4,
    SW_SHOW = 5,
    SWP_NOSIZE = 0x0001,
    SWP_NOMOVE = 0x0002,
    SWP_NOACTIVATE = 0x0010,
    TRANSPARENT = 1,
    DT_CENTER = 0x00000001,
    DT_VCENTER = 0x00000004,
    DT_SINGLELINE = 0x00000020,
    BUTTON_ID_BASE = 2000,
    INPUT_ID_BASE = 3000,
    SELECT_ID_BASE = 4000,
    SCROLL_ID_BASE = 5000,
    LIST_ID_BASE = 6000,
    MENU_COMMAND_BASE = 7000,
    TABLE_ID_BASE = 8000,
    BN_CLICKED = 0,
    EN_CHANGE = 0x0300,
    EN_UPDATE = 0x0400,
    CBN_SELCHANGE = 1,
    ES_AUTOHSCROLL = 0x0080,
    ES_MULTILINE = 0x0004,
    ES_AUTOVSCROLL = 0x0040,
    ES_READONLY = 0x0800,
    CBS_DROPDOWNLIST = 0x0003,
    CBS_NOINTEGRALHEIGHT = 0x0400,
    CB_ADDSTRING = 0x0143,
    CB_GETCURSEL = 0x0147,
    CB_SETCURSEL = 0x014E,
    LBS_NOTIFY = 0x0001,
    LBN_SELCHANGE = 1,
    LB_ADDSTRING = 0x0180,
    LB_RESETCONTENT = 0x0184,
    LB_GETCURSEL = 0x0188,
    LB_SETCURSEL = 0x0186,
    LVM_FIRST = 0x1000,
    LVM_SETBKCOLOR = LVM_FIRST + 1,
    LVM_DELETEALLITEMS = LVM_FIRST + 9,
    LVM_GETNEXTITEM = LVM_FIRST + 12,
    LVM_SETCOLUMNWIDTH = LVM_FIRST + 30,
    LVM_SETTEXTCOLOR = LVM_FIRST + 36,
    LVM_SETTEXTBKCOLOR = LVM_FIRST + 38,
    LVM_SETITEMSTATE = LVM_FIRST + 43,
    LVM_SETEXTENDEDLISTVIEWSTYLE = LVM_FIRST + 54,
    LVM_SETCOLUMNW = LVM_FIRST + 96,
    LVM_SETITEMW = LVM_FIRST + 76,
    LVM_INSERTITEMW = LVM_FIRST + 77,
    LVM_INSERTCOLUMNW = LVM_FIRST + 97,
    LVCF_WIDTH = 0x0002,
    LVCF_TEXT = 0x0004,
    LVIF_TEXT = 0x0001,
    LVIF_STATE = 0x0008,
    LVIS_SELECTED = 0x0002,
    LVNI_SELECTED = 0x0002,
    LVS_REPORT = 0x0001,
    LVS_SINGLESEL = 0x0004,
    LVS_SHOWSELALWAYS = 0x0008,
    LVS_EX_FULLROWSELECT = 0x00000020,
    LVN_ITEMCHANGED = (UINT)0xFFFFFF9BU,
    MF_STRING = 0x0000,
    TPM_LEFTALIGN = 0x0000,
    TPM_TOPALIGN = 0x0000,
    TPM_RETURNCMD = 0x0100,
    LAYOUT_COLUMN = 1,
    LAYOUT_ROW = 2,
    LAYOUT_CHILD_LABEL = 1,
    LAYOUT_CHILD_BUTTON = 2,
    LAYOUT_CHILD_NODE = 3,
    LAYOUT_CHILD_INPUT = 4,
    LAYOUT_CHILD_SELECT = 5,
    LAYOUT_CHILD_LIST = 6,
    LAYOUT_CHILD_TABLE = 7,
    UI_ALIGN_START = 0,
    UI_ALIGN_CENTER = 1,
    UI_ALIGN_END = 2,
    SIZE_MINIMIZED = 1,
    RDW_INVALIDATE = 0x0001,
    RDW_ERASE = 0x0004,
    RDW_ALLCHILDREN = 0x0080,
    RDW_UPDATENOW = 0x0100,
    ODS_SELECTED = 0x0001,
    ODS_DISABLED = 0x0004,
    ODS_FOCUS = 0x0010,
    ODS_HOTLIGHT = 0x0040,
    TME_LEAVE = 0x00000002,
    ICC_WIN95_CLASSES = 0x000000FF,
    TTF_IDISHWND = 0x0001,
    TTF_CENTERTIP = 0x0002,
    TTF_SUBCLASS = 0x0010,
    UI_STATE_BIND_CHECKED = 0,
    UI_STATE_BIND_INDEX = 1,
    UI_STATE_BIND_INPUT_LENGTH = 2,
    UI_STATE_BIND_COUNT = 3,
    UI_STATE_BIND_TEXT = 4,
    TTS_NOPREFIX = 0x0002,
    TTM_SETTIPBKCOLOR = 0x0413,
    TTM_SETTIPTEXTCOLOR = 0x0414,
    TTM_SETMAXTIPWIDTH = 0x0418,
    TTM_RELAYEVENT = 0x0407,
    TTM_GETTOOLCOUNT = 0x040D,
    TTM_ADDTOOLW = 0x0432,
    CW_USEDEFAULT = (int)0x80000000,
    IDC_ARROW = 32512,
};

static DesktopSpec desktop;
static int declared_theme_mode = 0;
static int active_theme_mode = 0;
static UINT_PTR gdiplus_token = 0;

enum {
    UI_THEME_LIGHT = 0,
    UI_THEME_DARK = 1,
    UI_THEME_SYSTEM = 2,
    UI_COLOR_WINDOW = 0,
    UI_COLOR_TOP_BAR = 1,
    UI_COLOR_PANEL = 2,
    UI_COLOR_PRIMARY = 3,
    UI_COLOR_SAFE = 4,
    UI_COLOR_STATUS = 5,
    UI_COLOR_TEXT = 6,
    UI_COLOR_SECONDARY_TEXT = 7,
    UI_COLOR_BORDER = 8,
};

static DWORD win32_color(UINT rgb) {
    return ((rgb & 0x000000FFU) << 16) | (rgb & 0x0000FF00U) | ((rgb & 0x00FF0000U) >> 16);
}

static int clamp_dimension(int value, int minimum, int maximum) {
    if (value < minimum) {
        return minimum;
    }
    if (value > maximum) {
        return maximum;
    }
    return value;
}

static UINT adjust_rgb(UINT rgb, int adjustment) {
    int red = clamp_dimension((int)((rgb >> 16) & 0xFFU) + adjustment, 0, 255);
    int green = clamp_dimension((int)((rgb >> 8) & 0xFFU) + adjustment, 0, 255);
    int blue = clamp_dimension((int)(rgb & 0xFFU) + adjustment, 0, 255);
    return ((UINT)red << 16) | ((UINT)green << 8) | (UINT)blue;
}

static int normalize_theme_mode(int mode) {
    /* System theme is a deterministic light fallback until the preview owns a
     * cross-platform system-appearance API. The source remains forward-ready. */
    return mode == UI_THEME_DARK ? UI_THEME_DARK : UI_THEME_LIGHT;
}

static UINT theme_color_for(int mode, int role) {
    int normalized_mode = normalize_theme_mode(mode);
    if (normalized_mode == UI_THEME_DARK) {
        switch (role) {
            case UI_COLOR_WINDOW: return 0x1E1E1EU;
            case UI_COLOR_TOP_BAR: return 0x252526U;
            case UI_COLOR_PANEL: return 0x2D2D30U;
            case UI_COLOR_PRIMARY: return 0x168EF5U;
            case UI_COLOR_SAFE: return 0x3A8D5DU;
            case UI_COLOR_STATUS: return 0x123F2AU;
            case UI_COLOR_TEXT: return 0xF3F4F6U;
            case UI_COLOR_SECONDARY_TEXT: return 0xB8BEC9U;
            case UI_COLOR_BORDER: return 0x4B5563U;
            default: return 0xF3F4F6U;
        }
    }
    switch (role) {
        case UI_COLOR_WINDOW: return 0xF6F6F9U;
        case UI_COLOR_TOP_BAR: return 0xF1F5F9U;
        case UI_COLOR_PANEL: return 0xFFFFFFU;
        case UI_COLOR_PRIMARY: return 0x168EF5U;
        case UI_COLOR_SAFE: return 0x18C964U;
        case UI_COLOR_STATUS: return 0xDCFCE7U;
        case UI_COLOR_TEXT: return 0x111827U;
        case UI_COLOR_SECONDARY_TEXT: return 0x6B7280U;
        case UI_COLOR_BORDER: return 0xCBD5E1U;
        default: return 0x111827U;
    }
}

UINT ui_theme_color(int role) {
    return theme_color_for(declared_theme_mode, role);
}

static UINT remap_theme_color(UINT color, int from_mode, int to_mode) {
    int role;
    for (role = UI_COLOR_WINDOW; role <= UI_COLOR_SECONDARY_TEXT; role += 1) {
        if (color == theme_color_for(from_mode, role)) {
            return theme_color_for(to_mode, role);
        }
    }
    return color;
}

static void replace_brush(HBRUSH *brush, UINT color) {
    if (*brush != 0) {
        DeleteObject(*brush);
    }
    *brush = CreateSolidBrush(win32_color(color));
}

static void apply_theme_change(int from_mode, int to_mode) {
    int index;
    if (from_mode == to_mode) {
        return;
    }
    desktop.background_color = remap_theme_color(desktop.background_color, from_mode, to_mode);
    replace_brush(&desktop.background_brush, desktop.background_color);
    replace_brush(&desktop.border_brush, theme_color_for(to_mode, UI_COLOR_BORDER));
    for (index = 0; index < desktop.label_count; index += 1) {
        LabelSpec *label = &desktop.labels[index];
        label->text_color = remap_theme_color(label->text_color, from_mode, to_mode);
        label->background_color = remap_theme_color(label->background_color, from_mode, to_mode);
        replace_brush(&label->brush, label->background_color);
    }
    for (index = 0; index < desktop.button_count; index += 1) {
        ButtonSpec *button = &desktop.buttons[index];
        button->text_color = remap_theme_color(button->text_color, from_mode, to_mode);
        button->background_color = remap_theme_color(button->background_color, from_mode, to_mode);
        replace_brush(&button->brush, button->background_color);
    }
    for (index = 0; index < desktop.input_count; index += 1) {
        InputSpec *input = &desktop.inputs[index];
        input->text_color = remap_theme_color(input->text_color, from_mode, to_mode);
        input->background_color = remap_theme_color(input->background_color, from_mode, to_mode);
        replace_brush(&input->brush, input->background_color);
    }
    for (index = 0; index < desktop.select_count; index += 1) {
        SelectSpec *select = &desktop.selects[index];
        select->text_color = remap_theme_color(select->text_color, from_mode, to_mode);
        select->background_color = remap_theme_color(select->background_color, from_mode, to_mode);
        replace_brush(&select->brush, select->background_color);
    }
    for (index = 0; index < desktop.scroll_count; index += 1) {
        ScrollSpec *scroll = &desktop.scrolls[index];
        scroll->text_color = remap_theme_color(scroll->text_color, from_mode, to_mode);
        scroll->background_color = remap_theme_color(scroll->background_color, from_mode, to_mode);
        replace_brush(&scroll->brush, scroll->background_color);
    }
    for (index = 0; index < desktop.list_count; index += 1) {
        ListSpec *list = &desktop.lists[index];
        list->text_color = remap_theme_color(list->text_color, from_mode, to_mode);
        list->background_color = remap_theme_color(list->background_color, from_mode, to_mode);
        replace_brush(&list->brush, list->background_color);
    }
    for (index = 0; index < desktop.table_count; index += 1) {
        TableSpec *table = &desktop.tables[index];
        table->text_color = remap_theme_color(table->text_color, from_mode, to_mode);
        table->background_color = remap_theme_color(table->background_color, from_mode, to_mode);
        replace_brush(&table->brush, table->background_color);
        if (table->handle != 0) {
            SendMessageW(table->handle, LVM_SETBKCOLOR, 0,
                         (LPARAM)win32_color(table->background_color));
            SendMessageW(table->handle, LVM_SETTEXTCOLOR, 0,
                         (LPARAM)win32_color(table->text_color));
            SendMessageW(table->handle, LVM_SETTEXTBKCOLOR, 0,
                         (LPARAM)win32_color(table->background_color));
        }
    }
    for (index = 0; index < desktop.tooltip_count; index += 1) {
        TooltipSpec *tooltip = &desktop.tooltips[index];
        tooltip->text_color = remap_theme_color(tooltip->text_color, from_mode, to_mode);
        tooltip->background_color = remap_theme_color(tooltip->background_color, from_mode, to_mode);
        if (tooltip->handle != 0) {
            SendMessageW(tooltip->handle, TTM_SETTIPBKCOLOR,
                         (WPARAM)win32_color(tooltip->background_color), 0);
            SendMessageW(tooltip->handle, TTM_SETTIPTEXTCOLOR,
                         (WPARAM)win32_color(tooltip->text_color), 0);
        }
    }
    if (desktop.window != 0) {
        RedrawWindow(desktop.window, 0, 0,
                     RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW);
    }
}

void ui_theme(int mode) {
    int next_mode = normalize_theme_mode(mode);
    if (desktop.window != 0 && desktop.background_brush != 0) {
        apply_theme_change(active_theme_mode, next_mode);
        active_theme_mode = next_mode;
    }
    declared_theme_mode = mode;
}

static int startup_error(int code, LPCWSTR text) {
    MessageBoxW(0, text, L"Jadren Desktop Preview", 0);
    ExitProcess((UINT)code);
    return code;
}

static void copy_utf8(JadrenString value, wchar_t *target, int capacity) {
    int length;
    if (capacity <= 0) {
        return;
    }
    if (value.data == 0 || value.length <= 0) {
        target[0] = L'\0';
        return;
    }
    length = value.length > 4096 ? 4096 : (int)value.length;
    length = MultiByteToWideChar(CP_UTF8, 0, value.data, length, target, capacity - 1);
    target[length > 0 ? length : 0] = L'\0';
}

static int wide_length(const wchar_t *value) {
    int length = 0;
    while (value != 0 && value[length] != L'\0') {
        length += 1;
    }
    return length;
}

static int system_icon_name_equals(JadrenString value, const char *name) {
    LONGLONG index = 0;
    if (value.data == 0 || name == 0) {
        return 0;
    }
    while (name[index] != '\0') {
        index += 1;
    }
    if (value.length != index) {
        return 0;
    }
    for (index = 0; index < value.length; index += 1) {
        if (value.data[index] != name[index]) {
            return 0;
        }
    }
    return 1;
}

/* Segoe MDL2 Assets names are kept as a small stable Jadren vocabulary. The
 * resulting private-use glyph is still an ordinary UTF-16 button label, so
 * unknown names continue to work as normal text/Unicode labels. */
static int copy_system_icon(JadrenString value, wchar_t *target, int capacity) {
    static const char *names[] = {
        "search", "settings", "help", "download", "save", "close", "menu",
        "refresh", "play", "stop", "info", "warning", "folder", "file",
    };
    static const wchar_t glyphs[] = {
        (wchar_t)0xE721, (wchar_t)0xE713, (wchar_t)0xE897, (wchar_t)0xE896,
        (wchar_t)0xE74E, (wchar_t)0xE8BB, (wchar_t)0xE700, (wchar_t)0xE72C,
        (wchar_t)0xE768, (wchar_t)0xE71A, (wchar_t)0xE946, (wchar_t)0xE7BA,
        (wchar_t)0xE8B7, (wchar_t)0xE7C3,
    };
    int index;
    if (target == 0 || capacity < 2) {
        return 0;
    }
    for (index = 0; index < (int)(sizeof(names) / sizeof(names[0])); index += 1) {
        if (system_icon_name_equals(value, names[index])) {
            target[0] = glyphs[index];
            target[1] = L'\0';
            return 1;
        }
    }
    return 0;
}

/* Copy a native UTF-16 model value into a caller-owned UTF-8 slice. The
 * two-pass conversion guarantees that a small output buffer produces no
 * partial value and that the returned length is exact. */
static unsigned __int64 copy_wide_utf8(const wchar_t *value,
                                       unsigned char *output_data,
                                       unsigned __int64 output_length) {
    int wide_count;
    int required;
    int capacity;
    int copied;
    if (value == 0 || output_data == 0 || output_length == 0) {
        return 0;
    }
    wide_count = wide_length(value);
    capacity = output_length > 0x7FFFFFFFULL ? 0x7FFFFFFF : (int)output_length;
    if (wide_count <= 0 || capacity <= 0) {
        return 0;
    }
    required = WideCharToMultiByte(CP_UTF8, 0, value, wide_count,
                                   0, 0, 0, 0);
    if (required <= 0 || required > capacity) {
        return 0;
    }
    copied = WideCharToMultiByte(CP_UTF8, 0, value, wide_count,
                                 (char *)output_data, capacity, 0, 0);
    return copied == required ? (unsigned __int64)copied : 0;
}

static unsigned __int64 wide_utf8_length(const wchar_t *value) {
    int wide_count;
    int bytes;
    if (value == 0) return 0;
    wide_count = wide_length(value);
    if (wide_count <= 0) return 0;
    bytes = WideCharToMultiByte(CP_UTF8, 0, value, wide_count, 0, 0, 0, 0);
    return bytes > 0 ? (unsigned __int64)bytes : 0;
}

static void copy_wide_text(const wchar_t *source, wchar_t *target, int capacity) {
    int index;
    if (target == 0 || capacity <= 0) return;
    for (index = 0; index + 1 < capacity && source != 0 && source[index] != L'\0'; index += 1) {
        target[index] = source[index];
    }
    target[index] = L'\0';
}

static void copy_utf8_bytes(const char *source, unsigned __int64 length,
                            char *target, int capacity) {
    unsigned __int64 index;
    unsigned __int64 limit;
    if (target == 0 || capacity <= 0) return;
    limit = (unsigned __int64)(capacity - 1);
    if (length > limit) length = limit;
    for (index = 0; index < length; index += 1) {
        target[index] = source == 0 ? 0 : source[index];
    }
    target[length] = '\0';
}

static void sync_input_text(InputSpec *input) {
    if (input != 0 && desktop.window != 0 && input->id != 0) {
        /* Layout/recreation can leave a still-valid HWND in the descriptor.
         * Resolve by the stable dialog control ID at every read so the
         * runtime never samples a different EDIT after a native refresh. */
        HWND current = GetDlgItem(desktop.window, (int)input->id);
        if (current != 0) input->handle = current;
    }
    if (input != 0 && input->handle != 0) {
        /* WM_GETTEXT is evaluated by the EDIT instance itself after an
         * explicitly marshalled system WM_SETTEXT. Never dereference the
         * sender-owned WM_SETTEXT lParam in this process. */
        SendMessageW(input->handle, 0x000D, (WPARAM)384,
                     (LPARAM)(unsigned __int64)input->text);
        input->text[383] = L'\0';
    }
}

static int input_utf8_length(InputSpec *input) {
    int length;
    int bytes;
    if (input == 0) {
        return 0;
    }
    sync_input_text(input);
    length = wide_length(input->text);
    if (length <= 0) {
        return 0;
    }
    bytes = WideCharToMultiByte(CP_UTF8, 0, input->text, length, 0, 0, 0, 0);
    return bytes > 0 ? bytes : 0;
}

static BOOL has_svg_extension(const wchar_t *path) {
    int length = wide_length(path);
    if (length < 4) {
        return 0;
    }
    return path[length - 4] == L'.' &&
           (path[length - 3] == L's' || path[length - 3] == L'S') &&
           (path[length - 2] == L'v' || path[length - 2] == L'V') &&
           (path[length - 1] == L'g' || path[length - 1] == L'G');
}

static BOOL is_absolute_path(const wchar_t *path) {
    return path != 0 &&
           ((path[0] != L'\0' && path[1] == L':') ||
            (path[0] == L'\\' && path[1] == L'\\'));
}

static void append_wide(wchar_t *target, int capacity, const wchar_t *suffix) {
    int target_length = wide_length(target);
    int index = 0;
    while (suffix != 0 && suffix[index] != L'\0' && target_length + index + 1 < capacity) {
        target[target_length + index] = suffix[index];
        index += 1;
    }
    target[target_length + index] = L'\0';
}

static void resolve_image_path(const wchar_t *path, wchar_t *resolved, int capacity) {
    int index = 0;
    int base_length;
    if (capacity <= 0) {
        return;
    }
    resolved[0] = L'\0';
    if (path == 0 || path[0] == L'\0') {
        return;
    }
    if (is_absolute_path(path)) {
        while (path[index] != L'\0' && index + 1 < capacity) {
            resolved[index] = path[index];
            index += 1;
        }
        resolved[index] = L'\0';
    } else {
        DWORD copied = GetModuleFileNameW(0, resolved, (DWORD)capacity);
        if (copied == 0) {
            return;
        }
        resolved[copied < (DWORD)capacity ? copied : capacity - 1] = L'\0';
        base_length = wide_length(resolved);
        while (base_length > 0 && resolved[base_length - 1] != L'\\' &&
               resolved[base_length - 1] != L'/') {
            base_length -= 1;
        }
        resolved[base_length] = L'\0';
        append_wide(resolved, capacity, path);
    }
    if (has_svg_extension(path)) {
        append_wide(resolved, capacity, L".png");
    }
}

static void reset_desktop(void) {
    int index;
    desktop.width = 720;
    desktop.height = 430;
    desktop.min_width = 360;
    desktop.min_height = 240;
    desktop.max_width = 1600;
    desktop.max_height = 1000;
    desktop.background_color = 0xF6F8FCU;
    desktop.base_client_width = 0;
    desktop.base_client_height = 0;
    desktop.label_count = 0;
    desktop.status_index = -1;
    desktop.button_count = 0;
    desktop.input_count = 0;
    desktop.select_count = 0;
    desktop.scroll_count = 0;
    desktop.list_count = 0;
    desktop.table_count = 0;
    desktop.input_app_binding_count = 0;
    desktop.menu_count = 0;
    desktop.image_count = 0;
    desktop.tooltip_count = 0;
    desktop.state_binding_count = 0;
    desktop.layout_count = 0;
    desktop.layout_depth = 0;
    desktop.events_ready = 0;
    desktop.resize_event_id = 0;
    desktop.close_event_id = 0;
    desktop.close_event_sent = 0;
    for (index = 0; index < 32; index += 1) {
        desktop.state_slots[index] = 0;
        desktop.state_text[index][0] = L'\0';
    }
    desktop.background_brush = 0;
    desktop.border_brush = 0;
    desktop.window = 0;
    desktop.title[0] = L'\0';
}

static LabelSpec *add_label(JadrenString text, int x, int y, int width, int height,
                            UINT text_color, UINT background_color, int corner_radius,
                            BOOL fills_width) {
    LabelSpec *label;
    if (desktop.label_count >= 32) {
        return 0;
    }
    label = &desktop.labels[desktop.label_count];
    desktop.label_count += 1;
    copy_utf8(text, label->text, 384);
    label->x = x;
    label->y = y;
    label->width = clamp_dimension(width, 1, 1600);
    label->height = clamp_dimension(height, 1, 600);
    label->text_color = text_color;
    label->background_color = background_color;
    label->corner_radius = clamp_dimension(corner_radius, 0, 128);
    label->fills_width = fills_width;
    label->layout_managed = 0;
    label->handle = 0;
    label->brush = 0;
    return label;
}

void jadren_win32_window_begin(const char *title_data, unsigned __int64 title_length,
                               int width, int height, UINT background_color) {
    JadrenString title = {title_data, (LONGLONG)title_length};
    reset_desktop();
    copy_utf8(title, desktop.title, 256);
    if (desktop.title[0] == L'\0') {
        copy_utf8((JadrenString){"Jadren Desktop", 14}, desktop.title, 256);
    }
    desktop.width = clamp_dimension(width, 360, 1600);
    desktop.height = clamp_dimension(height, 240, 1000);
    desktop.min_width = 360;
    desktop.min_height = 240;
    desktop.max_width = 1600;
    desktop.max_height = 1000;
    desktop.background_color = background_color;
}

void jadren_win32_label(const char *text_data, unsigned __int64 text_length,
                        int x, int y, int width, int height,
                        UINT text_color, UINT background_color) {
    add_label((JadrenString){text_data, (LONGLONG)text_length}, x, y, width, height,
              text_color, background_color, 0, 1);
}

void jadren_win32_status(const char *text_data, unsigned __int64 text_length,
                         int x, int y, int width, int height,
                         UINT text_color, UINT background_color) {
    LabelSpec *status = add_label((JadrenString){text_data, (LONGLONG)text_length}, x, y,
                                  width, height, text_color, background_color, 0, 1);
    if (status != 0) {
        desktop.status_index = desktop.label_count - 1;
    }
}

static ButtonSpec *add_button(JadrenString label, JadrenString action, int x, int y, int width,
                              int height, UINT text_color, UINT background_color,
                              int corner_radius, BOOL closes_window, BOOL flat_style,
                              BOOL right_anchored, BOOL distributes_horizontally,
                              BOOL is_menu_item, BOOL toggles, BOOL disabled) {
    ButtonSpec *button;
    if (desktop.button_count >= 24) {
        return 0;
    }
    button = &desktop.buttons[desktop.button_count];
    desktop.button_count += 1;
    copy_utf8(label, button->label, 160);
    copy_utf8(action, button->action, 384);
    button->x = x;
    button->y = y;
    button->current_x = x;
    button->current_y = y;
    button->width = clamp_dimension(width, 1, 800);
    button->height = clamp_dimension(height, 1, 240);
    button->text_color = text_color;
    button->background_color = background_color;
    button->corner_radius = clamp_dimension(corner_radius, 0, 128);
    button->id = BUTTON_ID_BASE + (UINT)desktop.button_count;
    button->closes_window = closes_window;
    button->flat_style = flat_style;
    button->right_anchored = right_anchored;
    button->distributes_horizontally = distributes_horizontally;
    button->is_menu_item = is_menu_item;
    button->toggles = toggles;
    button->active = 0;
    button->disabled = disabled;
    button->system_icon = 0;
    button->hovered = 0;
    button->pressed = 0;
    button->tracks_mouse = 0;
    button->event_id = 0;
    button->emits_event = 0;
    button->layout_managed = 0;
    button->is_checkbox = 0;
    button->is_switch = 0;
    button->default_proc = 0;
    button->handle = 0;
    button->brush = 0;
    button->font = 0;
    return button;
}

void jadren_win32_button(const char *label_data, unsigned __int64 label_length,
                         const char *action_data, unsigned __int64 action_length,
                         int x, int y, int width, int height,
                         UINT text_color, UINT background_color) {
    add_button((JadrenString){label_data, (LONGLONG)label_length},
               (JadrenString){action_data, (LONGLONG)action_length},
               x, y, width, height, text_color, background_color, 0, 0, 0, 0, 1, 0, 0, 0);
}

void jadren_win32_close_button(const char *label_data, unsigned __int64 label_length,
                               int x, int y, int width, int height,
                               UINT text_color, UINT background_color) {
    add_button((JadrenString){label_data, (LONGLONG)label_length},
               (JadrenString){0, 0}, x, y, width, height, text_color, background_color,
               0, 1, 0, 0, 1, 0, 0, 0);
}

/*
 * Jadren UI builtins. They are direct language functions, so application
 * source has no Win32 declarations. The main window intentionally uses the
 * native Windows frame; corner radii belong to child UI controls.
 */
void ui_window(const char *title_data, unsigned __int64 title_length,
               int width, int height, UINT background_color) {
    jadren_win32_window_begin(title_data, title_length, width, height, background_color);
}

void ui_top_bar(int height, UINT background_color) {
    /* Must be declared before its menu and icon controls so it stays behind them. */
    add_label((JadrenString){0, 0}, 0, 0, desktop.width, height,
              0, background_color, 0, 1);
}

void ui_label(const char *text_data, unsigned __int64 text_length,
              int x, int y, int width, int height,
              UINT text_color, UINT background_color, int corner_radius) {
    add_label((JadrenString){text_data, (LONGLONG)text_length}, x, y, width, height,
              text_color, background_color, corner_radius, 1);
}

void ui_text(const char *text_data, unsigned __int64 text_length,
             int x, int y, int width, int height,
             UINT text_color, UINT background_color, int corner_radius) {
    add_label((JadrenString){text_data, (LONGLONG)text_length}, x, y, width, height,
              text_color, background_color, corner_radius, 0);
}

void ui_status(const char *text_data, unsigned __int64 text_length,
               int x, int y, int width, int height,
               UINT text_color, UINT background_color, int corner_radius) {
    LabelSpec *status = add_label((JadrenString){text_data, (LONGLONG)text_length}, x, y,
                                  width, height, text_color, background_color, corner_radius, 1);
    if (status != 0) {
        desktop.status_index = desktop.label_count - 1;
    }
}

void ui_button(const char *label_data, unsigned __int64 label_length,
               const char *action_data, unsigned __int64 action_length,
               int x, int y, int width, int height,
               UINT text_color, UINT background_color, int corner_radius) {
    add_button((JadrenString){label_data, (LONGLONG)label_length},
               (JadrenString){action_data, (LONGLONG)action_length},
               x, y, width, height, text_color, background_color,
               corner_radius, 0, 0, 0, 1, 0, 0, 0);
}

/* An event button delegates behaviour to the explicit Jadren callback. The
 * event id is application-defined and also addresses this control for the
 * dynamic `ui_set_button_*` functions. */
void ui_event_button(const char *label_data, unsigned __int64 label_length,
                     int event_id, int x, int y, int width, int height,
                     UINT text_color, UINT background_color, int corner_radius) {
    ButtonSpec *button = add_button(
        (JadrenString){label_data, (LONGLONG)label_length}, (JadrenString){0, 0},
        x, y, width, height, text_color, background_color,
        corner_radius, 0, 0, 0, 1, 0, 0, 0);
    if (button != 0) {
        button->event_id = event_id;
        button->emits_event = 1;
    }
}

void ui_close_button(const char *label_data, unsigned __int64 label_length,
                     int x, int y, int width, int height,
                     UINT text_color, UINT background_color, int corner_radius) {
    add_button((JadrenString){label_data, (LONGLONG)label_length},
               (JadrenString){0, 0}, x, y, width, height,
               text_color, background_color, corner_radius, 1, 0, 0, 1, 0, 0, 0);
}

void ui_menu_item(const char *label_data, unsigned __int64 label_length,
                  const char *action_data, unsigned __int64 action_length,
                  int x, int y, int width, int height,
                  UINT text_color, UINT background_color, int corner_radius) {
    add_button((JadrenString){label_data, (LONGLONG)label_length},
               (JadrenString){action_data, (LONGLONG)action_length},
               x, y, width, height, text_color, background_color,
               corner_radius, 0, 1, 0, 0, 1, 0, 0);
}

static MenuSpec *menu_for_id(int menu_id) {
    int index;
    for (index = 0; index < desktop.menu_count; index += 1) {
        if (desktop.menus[index].menu_id == menu_id) {
            return &desktop.menus[index];
        }
    }
    return 0;
}

static MenuSpec *menu_for_button_id(UINT button_id) {
    int index;
    for (index = 0; index < desktop.menu_count; index += 1) {
        if (desktop.menus[index].button_id == button_id) {
            return &desktop.menus[index];
        }
    }
    return 0;
}

void ui_menu(const char *label_data, unsigned __int64 label_length,
             int menu_id, int x, int y, int width, int height,
             UINT text_color, UINT background_color, int corner_radius) {
    ButtonSpec *button;
    MenuSpec *menu;
    if (desktop.menu_count >= 8) {
        return;
    }
    button = add_button((JadrenString){label_data, (LONGLONG)label_length},
                        (JadrenString){0, 0}, x, y, width, height,
                        text_color, background_color, corner_radius,
                        0, 1, 0, 0, 1, 0, 0);
    if (button == 0) {
        return;
    }
    menu = &desktop.menus[desktop.menu_count];
    desktop.menu_count += 1;
    menu->option_count = 0;
    menu->menu_id = menu_id;
    menu->button_id = button->id;
}

void ui_menu_option(int menu_id, const char *label_data, unsigned __int64 label_length,
                    int event_id) {
    MenuSpec *menu = menu_for_id(menu_id);
    if (menu == 0 || menu->option_count >= 12) {
        return;
    }
    copy_utf8((JadrenString){label_data, (LONGLONG)label_length},
              menu->options[menu->option_count], 160);
    menu->option_event_ids[menu->option_count] = event_id;
    menu->option_count += 1;
}

/* Attach a small hover hint to an existing event control. The hint is declared
 * in Jadren next to the control; it does not need native coordinates because
 * the runtime follows the control when a responsive layout moves it. */
void ui_tooltip(int event_id, const char *text_data, unsigned __int64 text_length,
                int width, int height, UINT text_color, UINT background_color,
                int corner_radius) {
    TooltipSpec *tooltip;
    if (desktop.tooltip_count >= 16) {
        return;
    }
    tooltip = &desktop.tooltips[desktop.tooltip_count];
    desktop.tooltip_count += 1;
    copy_utf8((JadrenString){text_data, (LONGLONG)text_length}, tooltip->text, 384);
    tooltip->event_id = event_id;
    tooltip->width = clamp_dimension(width, 80, 480);
    tooltip->height = clamp_dimension(height, 22, 120);
    tooltip->text_color = text_color;
    tooltip->background_color = background_color;
    tooltip->corner_radius = corner_radius;
    tooltip->handle = 0;
}

/* Load a PNG file at run time. Coordinates are client-relative and the image
 * is painted below native controls, which keeps the control tree stable during
 * rapid resize. A missing or unsupported asset is reported during ui_run. */
void ui_image(const char *path_data, unsigned __int64 path_length,
              int x, int y, int width, int height) {
    ImageSpec *image;
    if (desktop.image_count >= 12) {
        return;
    }
    image = &desktop.images[desktop.image_count];
    desktop.image_count += 1;
    copy_utf8((JadrenString){path_data, (LONGLONG)path_length}, image->path, 520);
    image->x = x;
    image->y = y;
    image->width = clamp_dimension(width, 1, 1600);
    image->height = clamp_dimension(height, 1, 1200);
    image->image = 0;
}

static void sync_app_bindings_from_native(void);

static void show_popup_menu(HWND window, MenuSpec *menu) {
    HMENU native_menu;
    POINT point;
    UINT command;
    int menu_index;
    int option_index;
    if (menu == 0 || menu->option_count <= 0) {
        return;
    }
    native_menu = CreatePopupMenu();
    if (native_menu == 0) {
        return;
    }
    menu_index = (int)(menu - desktop.menus);
    for (option_index = 0; option_index < menu->option_count; option_index += 1) {
        AppendMenuW(native_menu, MF_STRING,
                    (UINT_PTR)(MENU_COMMAND_BASE + menu_index * 16 + option_index),
                    menu->options[option_index]);
    }
    if (GetCursorPos(&point) != 0) {
        /* Win32 popup menus need foreground ownership and a queued null
         * message after dismissal. Without this pair some compositors can
         * retain a selected popup row until the next unrelated repaint. */
        SetForegroundWindow(window);
        command = TrackPopupMenu(native_menu, TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RETURNCMD,
                                 point.x, point.y, 0, window, 0);
#if JADREN_UI_HAS_EVENT_CALLBACK
        for (option_index = 0; option_index < menu->option_count; option_index += 1) {
            if (command == (UINT)(MENU_COMMAND_BASE + menu_index * 16 + option_index)) {
                sync_app_bindings_from_native();
                jadren_ui_on_click(menu->option_event_ids[option_index]);
                refresh_app_bindings();
                break;
            }
        }
#endif
    }
    DestroyMenu(native_menu);
    PostMessageW(window, 0, 0, 0);
    RedrawWindow(window, 0, 0,
                 RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW);
}

void ui_icon_button(const char *icon_data, unsigned __int64 icon_length,
                    const char *action_data, unsigned __int64 action_length,
                    int right_offset, int y, int width, int height,
                    UINT text_color, UINT background_color, int corner_radius) {
    ButtonSpec *button;
    JadrenString icon = {icon_data, (LONGLONG)icon_length};
    /* right_offset keeps toolbar icons pinned to the right while resizing. */
    button = add_button(icon,
                        (JadrenString){action_data, (LONGLONG)action_length},
                        right_offset, y, width, height, text_color, background_color,
                        corner_radius, 0, 1, 1, 0, 0, 0, 0);
    if (button != 0) {
        button->system_icon = copy_system_icon(icon, button->label, 160);
    }
}

void ui_toggle_button(const char *label_data, unsigned __int64 label_length,
                      const char *action_data, unsigned __int64 action_length,
                      int x, int y, int width, int height,
                      UINT text_color, UINT background_color, int corner_radius) {
    add_button((JadrenString){label_data, (LONGLONG)label_length},
               (JadrenString){action_data, (LONGLONG)action_length},
               x, y, width, height, text_color, background_color,
               corner_radius, 0, 0, 0, 1, 0, 1, 0);
}

void ui_disabled_button(const char *label_data, unsigned __int64 label_length,
                        int x, int y, int width, int height,
                        UINT text_color, UINT background_color, int corner_radius) {
    add_button((JadrenString){label_data, (LONGLONG)label_length},
               (JadrenString){0, 0}, x, y, width, height,
               text_color, background_color, corner_radius, 0, 0, 0, 1, 0, 0, 1);
}

static void redraw_button(ButtonSpec *button);
static void sync_state_bindings_for_event(int event_id);
static void sync_checkbox_app_state(ButtonSpec *button);
static void refresh_checkbox_from_app_state(ButtonSpec *button);
static void sync_select_app_state(SelectSpec *select);
static void refresh_select_from_app_state(SelectSpec *select);
static void sync_list_app_state(ListSpec *list);
static void refresh_list_from_app_state(ListSpec *list);
static void sync_table_app_state(TableSpec *table);
static void refresh_table_from_app_state(TableSpec *table);
static void sync_app_bindings_from_native(void);
/* Retained node binding can attach to an already-loaded model without
 * overwriting it with the control's placeholder value. Legacy event-id
 * bindings keep their original initialise-from-control behaviour. */
static int portable_ui_preserve_app_binding;

static ButtonSpec *event_toggle_for_id(int event_id) {
    int index;
    for (index = 0; index < desktop.button_count; index += 1) {
        ButtonSpec *button = &desktop.buttons[index];
        if (button->emits_event && button->event_id == event_id &&
            (button->is_checkbox || button->is_switch)) {
            return button;
        }
    }
    return 0;
}

void ui_checkbox(const char *label_data, unsigned __int64 label_length,
                 int event_id, int x, int y, int width, int height,
                 UINT text_color, UINT background_color, int corner_radius) {
    ButtonSpec *button = add_button(
        (JadrenString){label_data, (LONGLONG)label_length}, (JadrenString){0, 0},
        x, y, width, height, text_color, background_color,
        corner_radius, 0, 1, 0, 0, 0, 1, 0);
    if (button != 0) {
        button->event_id = event_id;
        button->emits_event = 1;
        button->is_checkbox = 1;
    }
}

void ui_switch(const char *label_data, unsigned __int64 label_length,
               int event_id, int x, int y, int width, int height,
               UINT text_color, UINT background_color, int corner_radius) {
    ButtonSpec *button = add_button(
        (JadrenString){label_data, (LONGLONG)label_length}, (JadrenString){0, 0},
        x, y, width, height, text_color, background_color,
        corner_radius, 0, 1, 0, 0, 0, 1, 0);
    if (button != 0) {
        button->event_id = event_id;
        button->emits_event = 1;
        button->is_switch = 1;
    }
}

int ui_checked(int event_id) {
    ButtonSpec *button = event_toggle_for_id(event_id);
    return button != 0 && button->active ? 1 : 0;
}

void ui_set_checked(int event_id, int checked) {
    ButtonSpec *button = event_toggle_for_id(event_id);
    if (button == 0) {
        return;
    }
    button->active = checked != 0;
    sync_checkbox_app_state(button);
    sync_state_bindings_for_event(event_id);
    redraw_button(button);
}

static InputSpec *input_for_event_id(int event_id) {
    int index;
    for (index = 0; index < desktop.input_count; index += 1) {
        if (desktop.inputs[index].event_id == event_id) {
            if ((desktop.inputs[index].handle == 0 ||
                 !IsWindow(desktop.inputs[index].handle)) && desktop.window != 0) {
                desktop.inputs[index].handle =
                    GetDlgItem(desktop.window, (int)desktop.inputs[index].id);
            }
            return &desktop.inputs[index];
        }
    }
    return 0;
}

static InputSpec *input_for_handle(HWND handle) {
    int index;
    for (index = 0; index < desktop.input_count; index += 1) {
        if (desktop.inputs[index].handle == handle) {
            return &desktop.inputs[index];
        }
    }
    return 0;
}

static InputAppBinding *input_app_binding_for_event_id(int event_id) {
    int index;
    for (index = 0; index < desktop.input_app_binding_count; index += 1) {
        if (desktop.input_app_bindings[index].event_id == event_id) {
            return &desktop.input_app_bindings[index];
        }
    }
    return 0;
}

static void sync_input_app_state(InputSpec *input) {
#if JADREN_UI_HAS_FILE_RUNTIME
    InputAppBinding *binding;
    unsigned char output[256];
    unsigned __int64 copied;
    if (input == 0) return;
    binding = input_app_binding_for_event_id(input->event_id);
    if (binding == 0 || binding->kind != JADREN_APP_BIND_TEXT) return;
    sync_input_text(input);
    copied = copy_wide_utf8(input->text, output, sizeof(output));
    app_state_set_text(binding->key, binding->key_length,
                       (const char *)output, copied);
#else
    (void)input;
#endif
}

static void refresh_input_from_app_state(InputSpec *input) {
#if JADREN_UI_HAS_FILE_RUNTIME
    InputAppBinding *binding;
    unsigned char output[256];
    unsigned __int64 copied;
    if (input == 0) return;
    binding = input_app_binding_for_event_id(input->event_id);
    if (binding == 0 || binding->kind != JADREN_APP_BIND_TEXT) return;
    copied = app_state_read_text(binding->key, binding->key_length,
                                 output, sizeof(output));
    copy_utf8((JadrenString){(const char *)output, (LONGLONG)copied},
              input->text, 384);
    if (input->handle != 0) {
        /* The internal message never invokes Jadren callbacks. Send it
         * synchronously so a later native edit cannot race a stale queued
         * projection from an earlier refresh. */
        SendMessageW(input->handle, WM_JADREN_SET_INPUT_TEXT, 0,
                     (LPARAM)(unsigned __int64)input->text);
        UpdateWindow(input->handle);
    }
    sync_state_bindings_for_event(input->event_id);
#else
    (void)input;
#endif
}

void ui_text_input(const char *text_data, unsigned __int64 text_length,
                   int event_id, int x, int y, int width, int height,
                   UINT text_color, UINT background_color, int corner_radius) {
    InputSpec *input;
    if (desktop.input_count >= 12) {
        return;
    }
    input = &desktop.inputs[desktop.input_count];
    desktop.input_count += 1;
    copy_utf8((JadrenString){text_data, (LONGLONG)text_length}, input->text, 384);
    input->x = x;
    input->y = y;
    input->width = clamp_dimension(width, 1, 1600);
    input->height = clamp_dimension(height, 1, 240);
    input->text_color = text_color;
    input->background_color = background_color;
    input->corner_radius = clamp_dimension(corner_radius, 0, 128);
    input->event_id = event_id;
    input->id = INPUT_ID_BASE + (UINT)desktop.input_count;
    input->updating = 0;
    input->layout_managed = 0;
    input->default_proc = 0;
    input->handle = 0;
    input->brush = 0;
}

void ui_set_input_text(int event_id, const char *text_data, unsigned __int64 text_length) {
    InputSpec *input = input_for_event_id(event_id);
    if (input == 0) {
        return;
    }
    copy_utf8((JadrenString){text_data, (LONGLONG)text_length}, input->text, 384);
    if (input->handle != 0) {
        SendMessageW(input->handle, WM_JADREN_SET_INPUT_TEXT, 0,
                     (LPARAM)(unsigned __int64)input->text);
        UpdateWindow(input->handle);
    }
    if (!portable_ui_preserve_app_binding) {
#if JADREN_UI_HAS_FILE_RUNTIME
        InputAppBinding *binding = input_app_binding_for_event_id(event_id);
        unsigned char output[256];
        unsigned __int64 copied;
        if (binding != 0 && binding->kind == JADREN_APP_BIND_TEXT) {
            copied = copy_wide_utf8(input->text, output, sizeof(output));
            (void)app_state_set_text(binding->key, binding->key_length,
                                     (const char *)output, copied);
        }
#endif
    }
    sync_app_bindings_from_native();
    sync_state_bindings_for_event(event_id);
}

void ui_input_bind_app_state(int event_id, const char *key_data,
                             unsigned __int64 key_length) {
    InputAppBinding *binding = input_app_binding_for_event_id(event_id);
    InputSpec *input = input_for_event_id(event_id);
    if (input == 0 || key_data == 0 || key_length == 0 || key_length > 64) {
        return;
    }
    if (binding == 0) {
        if (desktop.input_app_binding_count >= 12) return;
        binding = &desktop.input_app_bindings[desktop.input_app_binding_count];
        desktop.input_app_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->kind = JADREN_APP_BIND_TEXT;
    binding->key_length = key_length;
    copy_utf8_bytes(key_data, key_length, binding->key, 65);
    /* Legacy event-id binding initializes the model from the declared input
     * value. The retained ui_app_* wrapper sets preserve mode so a model
     * loaded before the binding remains authoritative. */
    if (!portable_ui_preserve_app_binding) {
        sync_input_app_state(input);
    }
}

void ui_input_refresh_app_state(int event_id) {
    refresh_input_from_app_state(input_for_event_id(event_id));
}

/* Reads the current native EDIT value into a caller-owned Jadren Buffer.
 * The borrowed Buffer ABI is the same two-word pointer/length pair used by
 * String and Slice imports on Windows x64. The return value is the number of
 * UTF-8 bytes copied; the buffer is intentionally not NUL-terminated. */
unsigned __int64 ui_input_length(int event_id) {
    return (unsigned __int64)input_utf8_length(input_for_event_id(event_id));
}

unsigned __int64 ui_input_read(int event_id, unsigned char *output_data,
                               unsigned __int64 output_length) {
    InputSpec *input = input_for_event_id(event_id);
    int wide_count;
    int capacity;
    int bytes;
    if (input == 0 || output_data == 0 || output_length == 0) {
        return 0;
    }
    sync_input_text(input);
    wide_count = wide_length(input->text);
    capacity = output_length > 0x7FFFFFFFULL ? 0x7FFFFFFF : (int)output_length;
    if (wide_count <= 0 || capacity <= 0) {
        return 0;
    }
    bytes = WideCharToMultiByte(CP_UTF8, 0, input->text, wide_count,
                                (char *)output_data, capacity, 0, 0);
    return bytes > 0 ? (unsigned __int64)bytes : 0;
}

/* Exact read-back variant. Unlike ui_input_read, a valid empty control text
 * returns true and reports length zero. Neither caller-owned output is touched
 * unless the control and both output capacities are valid. */
int ui_input_read_exact(int event_id, unsigned char *output_data,
                        unsigned __int64 output_length,
                        unsigned __int64 *output_text_length,
                        unsigned __int64 output_text_length_capacity) {
    InputSpec *input = input_for_event_id(event_id);
    int wide_count;
    int required;
    int capacity;
    int bytes;
    if (input == 0 || output_text_length == 0 ||
        output_text_length_capacity == 0) {
        return 0;
    }
    sync_input_text(input);
    wide_count = wide_length(input->text);
    required = WideCharToMultiByte(CP_UTF8, 0, input->text, wide_count,
                                   0, 0, 0, 0);
    if (required < 0 || (required > 0 && output_data == 0)) {
        return 0;
    }
    if (output_length < (unsigned __int64)required) {
        return 0;
    }
    capacity = output_length > 0x7FFFFFFFULL ? 0x7FFFFFFF : (int)output_length;
    bytes = required > 0
        ? WideCharToMultiByte(CP_UTF8, 0, input->text, wide_count,
                              (char *)output_data, capacity, 0, 0)
        : 0;
    if (required > 0 && bytes != required) {
        return 0;
    }
    output_text_length[0] = (unsigned __int64)required;
    return 1;
}

void ui_set_input_enabled(int event_id, int enabled) {
    InputSpec *input = input_for_event_id(event_id);
    if (input != 0 && input->handle != 0) {
        EnableWindow(input->handle, enabled != 0);
    }
}

static SelectSpec *select_for_event_id(int event_id) {
    int index;
    for (index = 0; index < desktop.select_count; index += 1) {
        if (desktop.selects[index].event_id == event_id) {
            return &desktop.selects[index];
        }
    }
    return 0;
}

static SelectSpec *select_for_control_id(UINT id) {
    int index;
    for (index = 0; index < desktop.select_count; index += 1) {
        if (desktop.selects[index].id == id) {
            return &desktop.selects[index];
        }
    }
    return 0;
}

static void sync_checkbox_app_state(ButtonSpec *button) {
#if JADREN_UI_HAS_FILE_RUNTIME
    InputAppBinding *binding;
    if (button == 0) return;
    binding = input_app_binding_for_event_id(button->event_id);
    if (binding == 0 || binding->kind != JADREN_APP_BIND_BOOL) return;
    (void)app_state_set_bool(binding->key, binding->key_length,
                             (unsigned char)(button->active ? 1 : 0));
#else
    (void)button;
#endif
}

static void refresh_checkbox_from_app_state(ButtonSpec *button) {
#if JADREN_UI_HAS_FILE_RUNTIME
    InputAppBinding *binding;
    if (button == 0) return;
    binding = input_app_binding_for_event_id(button->event_id);
    if (binding == 0 || binding->kind != JADREN_APP_BIND_BOOL) return;
    button->active = app_state_get_bool(binding->key, binding->key_length) != 0;
    if (button->handle != 0) {
        SendMessageW(button->handle, 0x00F1,
                     (WPARAM)(button->active ? 1 : 0), 0);
    }
    sync_state_bindings_for_event(button->event_id);
    redraw_button(button);
#else
    (void)button;
#endif
}

static void sync_select_app_state(SelectSpec *select) {
#if JADREN_UI_HAS_FILE_RUNTIME
    InputAppBinding *binding;
    if (select == 0) return;
    binding = input_app_binding_for_event_id(select->event_id);
    if (binding == 0 || binding->kind != JADREN_APP_BIND_SELECT) return;
    (void)app_state_set_int(binding->key, binding->key_length,
                            (long long)select->selected_index);
#else
    (void)select;
#endif
}

static void refresh_select_from_app_state(SelectSpec *select) {
#if JADREN_UI_HAS_FILE_RUNTIME
    InputAppBinding *binding;
    long long value;
    if (select == 0) return;
    binding = input_app_binding_for_event_id(select->event_id);
    if (binding == 0 || binding->kind != JADREN_APP_BIND_SELECT) return;
    value = app_state_get_int(binding->key, binding->key_length);
    if (value < -1 || value >= (long long)select->option_count) {
        value = -1;
    }
    select->selected_index = (int)value;
    if (select->handle != 0) {
        SendMessageW(select->handle, CB_SETCURSEL,
                     (WPARAM)(unsigned __int64)(value < 0 ? -1 : value), 0);
    }
    sync_state_bindings_for_event(select->event_id);
#else
    (void)select;
#endif
}

int ui_checkbox_bind_app_state(int event_id, const char *key_data,
                               unsigned __int64 key_length) {
    InputAppBinding *binding = input_app_binding_for_event_id(event_id);
    ButtonSpec *button = event_toggle_for_id(event_id);
    if (button == 0 || key_data == 0 || key_length == 0 || key_length > 64) {
        return 0;
    }
    if (binding == 0) {
        if (desktop.input_app_binding_count >= 12) return 0;
        binding = &desktop.input_app_bindings[desktop.input_app_binding_count];
        desktop.input_app_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->kind = JADREN_APP_BIND_BOOL;
    binding->key_length = key_length;
    copy_utf8_bytes(key_data, key_length, binding->key, 65);
#if JADREN_UI_HAS_FILE_RUNTIME
    if (!portable_ui_preserve_app_binding &&
        !app_state_set_bool(binding->key, binding->key_length,
                            (unsigned char)(button->active ? 1 : 0))) {
        return 0;
    }
#endif
    if (!portable_ui_preserve_app_binding) {
        sync_checkbox_app_state(button);
    }
    return 1;
}

void ui_checkbox_refresh_app_state(int event_id) {
    refresh_checkbox_from_app_state(event_toggle_for_id(event_id));
}

int ui_select_bind_app_state(int event_id, const char *key_data,
                             unsigned __int64 key_length) {
    InputAppBinding *binding = input_app_binding_for_event_id(event_id);
    SelectSpec *select = select_for_event_id(event_id);
    if (select == 0 || key_data == 0 || key_length == 0 || key_length > 64) {
        return 0;
    }
    if (binding == 0) {
        if (desktop.input_app_binding_count >= 12) return 0;
        binding = &desktop.input_app_bindings[desktop.input_app_binding_count];
        desktop.input_app_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->kind = JADREN_APP_BIND_SELECT;
    binding->key_length = key_length;
    copy_utf8_bytes(key_data, key_length, binding->key, 65);
#if JADREN_UI_HAS_FILE_RUNTIME
    if (!portable_ui_preserve_app_binding &&
        !app_state_set_int(binding->key, binding->key_length,
                           (long long)select->selected_index)) {
        return 0;
    }
#endif
    if (!portable_ui_preserve_app_binding) {
        sync_select_app_state(select);
    }
    return 1;
}

void ui_select_refresh_app_state(int event_id) {
    refresh_select_from_app_state(select_for_event_id(event_id));
}

/* For a native COMBOBOX the window height is the *closed field plus the
 * popup list*, not only the 38px field visible in the Jadren source. Passing
 * just the closed height creates a functional arrow with a zero-height list. */
static int select_window_height(const SelectSpec *select) {
    int visible_options = select->option_count;
    if (visible_options < 1) {
        visible_options = 1;
    }
    if (visible_options > 8) {
        visible_options = 8;
    }
    return clamp_dimension(select->height + 4 + visible_options * 24, select->height + 4, 480);
}

void ui_select(int event_id, int x, int y, int width, int height,
               UINT text_color, UINT background_color) {
    SelectSpec *select;
    if (desktop.select_count >= 8) {
        return;
    }
    select = &desktop.selects[desktop.select_count];
    desktop.select_count += 1;
    select->option_count = 0;
    select->selected_index = -1;
    select->x = x;
    select->y = y;
    select->width = clamp_dimension(width, 1, 1600);
    select->height = clamp_dimension(height, 1, 240);
    select->text_color = text_color;
    select->background_color = background_color;
    select->corner_radius = 0;
    select->event_id = event_id;
    select->id = SELECT_ID_BASE + (UINT)desktop.select_count;
    select->layout_managed = 0;
    select->handle = 0;
    select->brush = 0;
}

void ui_select_option(int event_id, const char *text_data, unsigned __int64 text_length) {
    SelectSpec *select = select_for_event_id(event_id);
    if (select == 0 || select->option_count >= 12) {
        return;
    }
    copy_utf8((JadrenString){text_data, (LONGLONG)text_length},
              select->options[select->option_count], 160);
    select->option_count += 1;
}

int ui_select_index(int event_id) {
    SelectSpec *select = select_for_event_id(event_id);
    return select == 0 ? -1 : select->selected_index;
}

void ui_select_set_index(int event_id, int selected_index) {
    SelectSpec *select = select_for_event_id(event_id);
    if (select == 0 || selected_index < 0 || selected_index >= select->option_count) {
        return;
    }
    select->selected_index = selected_index;
    if (select->handle != 0) {
        SendMessageW(select->handle, CB_SETCURSEL, (WPARAM)(unsigned __int64)selected_index, 0);
    }
    sync_select_app_state(select);
    sync_state_bindings_for_event(event_id);
}

void ui_scroll_panel(const char *text_data, unsigned __int64 text_length,
                     int x, int y, int width, int height,
                     UINT text_color, UINT background_color, int corner_radius) {
    ScrollSpec *scroll;
    if (desktop.scroll_count >= 8) {
        return;
    }
    scroll = &desktop.scrolls[desktop.scroll_count];
    desktop.scroll_count += 1;
    copy_utf8((JadrenString){text_data, (LONGLONG)text_length}, scroll->text, 2048);
    scroll->x = x;
    scroll->y = y;
    scroll->width = clamp_dimension(width, 1, 1600);
    scroll->height = clamp_dimension(height, 1, 800);
    scroll->text_color = text_color;
    scroll->background_color = background_color;
    scroll->corner_radius = clamp_dimension(corner_radius, 0, 128);
    scroll->id = SCROLL_ID_BASE + (UINT)desktop.scroll_count;
    scroll->handle = 0;
    scroll->brush = 0;
}

static ListSpec *list_for_event_id(int event_id) {
    int index;
    for (index = 0; index < desktop.list_count; index += 1) {
        if (desktop.lists[index].event_id == event_id) {
            return &desktop.lists[index];
        }
    }
    return 0;
}

static ListSpec *list_for_control_id(UINT id) {
    int index;
    for (index = 0; index < desktop.list_count; index += 1) {
        if (desktop.lists[index].id == id) {
            return &desktop.lists[index];
        }
    }
    return 0;
}

/* Keep the native listbox synchronized after a callback mutates the Jadren
 * list model.  The model is bounded and caller-owned; rebuilding the small
 * control is deterministic and avoids stale rows during rapid updates. */
static void refresh_list_control(ListSpec *list) {
    int item_index;
    if (list == 0 || list->handle == 0) {
        return;
    }
    SendMessageW(list->handle, LB_RESETCONTENT, 0, 0);
    for (item_index = 0; item_index < list->item_count; item_index += 1) {
        SendMessageW(list->handle, LB_ADDSTRING, 0,
                     (LPARAM)(unsigned __int64)list->items[item_index]);
    }
    if (list->selected_index >= 0 && list->selected_index < list->item_count) {
        SendMessageW(list->handle, LB_SETCURSEL,
                     (WPARAM)(unsigned __int64)list->selected_index, 0);
    }
}

/* Copy one bounded application list into a native UI list. The refresh is
 * explicit: application code decides when a model mutation becomes visible,
 * so this remains deterministic and avoids an implicit reactive graph. */
static void refresh_list_from_app(ListSpec *list) {
#if JADREN_UI_HAS_FILE_RUNTIME
    unsigned char output[256];
    int item_index;
    int count;
    int selected_index;
    if (list == 0 || list->app_list_id < 0) {
        return;
    }
    selected_index = list->selected_index;
    count = app_list_count(list->app_list_id);
    if (count < 0) count = 0;
    if (count > 32) count = 32;
    list->item_count = count;
    list->selected_index = selected_index >= 0 && selected_index < count
        ? selected_index : -1;
    for (item_index = 0; item_index < count; item_index += 1) {
        unsigned __int64 copied = app_list_read_text(
            list->app_list_id, item_index, output, sizeof(output));
        copy_utf8((JadrenString){(const char *)output, (LONGLONG)copied},
                  list->items[item_index], 160);
    }
    refresh_list_control(list);
    sync_state_bindings_for_event(list->event_id);
#else
    (void)list;
#endif
}

void ui_list(int event_id, int x, int y, int width, int height,
             UINT text_color, UINT background_color) {
    ListSpec *list;
    if (desktop.list_count >= 8) {
        return;
    }
    list = &desktop.lists[desktop.list_count];
    desktop.list_count += 1;
    list->item_count = 0;
    list->selected_index = -1;
    list->x = x;
    list->y = y;
    list->width = clamp_dimension(width, 1, 1600);
    list->height = clamp_dimension(height, 1, 800);
    list->text_color = text_color;
    list->background_color = background_color;
    list->corner_radius = 0;
    list->event_id = event_id;
    list->id = LIST_ID_BASE + (UINT)desktop.list_count;
    list->app_list_id = -1;
    list->layout_managed = 0;
    list->handle = 0;
    list->brush = 0;
}

void ui_list_item(int event_id, const char *text_data, unsigned __int64 text_length) {
    ListSpec *list = list_for_event_id(event_id);
    if (list == 0 || list->item_count >= 32) {
        return;
    }
    copy_utf8((JadrenString){text_data, (LONGLONG)text_length},
              list->items[list->item_count], 160);
    list->item_count += 1;
    refresh_list_control(list);
    sync_state_bindings_for_event(event_id);
}

void ui_list_clear(int event_id) {
    ListSpec *list = list_for_event_id(event_id);
    if (list == 0) {
        return;
    }
    list->item_count = 0;
    list->selected_index = -1;
    sync_list_app_state(list);
    refresh_list_control(list);
    sync_state_bindings_for_event(event_id);
}

int ui_list_count(int event_id) {
    ListSpec *list = list_for_event_id(event_id);
    return list == 0 ? 0 : list->item_count;
}

unsigned __int64 ui_list_read_item(int event_id, int item_index,
                                   unsigned char *output_data,
                                   unsigned __int64 output_length) {
    ListSpec *list = list_for_event_id(event_id);
    if (list == 0 || item_index < 0 || item_index >= list->item_count) {
        return 0;
    }
    return copy_wide_utf8(list->items[item_index], output_data, output_length);
}

void ui_list_set_item(int event_id, int item_index,
                      const char *text_data, unsigned __int64 text_length) {
    ListSpec *list = list_for_event_id(event_id);
    if (list == 0 || item_index < 0 || item_index >= list->item_count) {
        return;
    }
    copy_utf8((JadrenString){text_data, (LONGLONG)text_length},
              list->items[item_index], 160);
    refresh_list_control(list);
    sync_state_bindings_for_event(event_id);
}

int ui_list_index(int event_id) {
    ListSpec *list = list_for_event_id(event_id);
    return list == 0 ? -1 : list->selected_index;
}

void ui_list_set_index(int event_id, int selected_index) {
    ListSpec *list = list_for_event_id(event_id);
    if (list == 0 || selected_index < -1 || selected_index >= list->item_count) {
        return;
    }
    list->selected_index = selected_index;
    if (list->handle != 0) {
        SendMessageW(list->handle, LB_SETCURSEL,
                     (WPARAM)(unsigned __int64)(selected_index < 0 ? -1 : selected_index), 0);
    }
    sync_list_app_state(list);
    sync_state_bindings_for_event(event_id);
}

void ui_list_bind_app(int event_id, int list_id) {
    ListSpec *list = list_for_event_id(event_id);
    if (list == 0 || list_id < 0) {
        return;
    }
    list->app_list_id = list_id;
    refresh_list_from_app(list);
}

void ui_list_refresh_app(int event_id) {
    refresh_list_from_app(list_for_event_id(event_id));
}

static TableSpec *table_for_event_id(int event_id) {
    int index;
    for (index = 0; index < desktop.table_count; index += 1) {
        if (desktop.tables[index].event_id == event_id) {
            return &desktop.tables[index];
        }
    }
    return 0;
}

static TableSpec *table_for_control_id(UINT id) {
    int index;
    for (index = 0; index < desktop.table_count; index += 1) {
        if (desktop.tables[index].id == id) {
            return &desktop.tables[index];
        }
    }
    return 0;
}

/* Rebuild the bounded report-view control after Jadren mutates its table
 * model.  The updating flag prevents native LVN_ITEMCHANGED notifications
 * emitted by this rebuild from re-entering the Jadren callback. */
static void refresh_table_control(TableSpec *table) {
    int row_index;
    int column_index;
    if (table == 0 || table->handle == 0) {
        return;
    }
    table->updating = 1;
    SendMessageW(table->handle, LVM_DELETEALLITEMS, 0, 0);
    if (table->column_count > 0) {
        for (row_index = 0; row_index < table->row_count; row_index += 1) {
            LVITEMW item = {0};
            item.mask = LVIF_TEXT;
            item.iItem = row_index;
            item.iSubItem = 0;
            item.pszText = table->cells[row_index][0];
            SendMessageW(table->handle, LVM_INSERTITEMW, 0, (LPARAM)&item);
            for (column_index = 1; column_index < table->column_count; column_index += 1) {
                item.mask = LVIF_TEXT;
                item.iItem = row_index;
                item.iSubItem = column_index;
                item.pszText = table->cells[row_index][column_index];
                SendMessageW(table->handle, LVM_SETITEMW, 0, (LPARAM)&item);
            }
        }
    }
    if (table->selected_row >= 0 && table->selected_row < table->row_count) {
        LVITEMW state = {0};
        state.mask = LVIF_STATE;
        state.iItem = table->selected_row;
        state.state = LVIS_SELECTED;
        state.stateMask = LVIS_SELECTED;
        SendMessageW(table->handle, LVM_SETITEMSTATE,
                     (WPARAM)(unsigned __int64)table->selected_row, (LPARAM)&state);
    }
    table->updating = 0;
}

static void update_table_column(TableSpec *table, int column_index, int was_declared) {
    LVCOLUMNW column = {0};
    if (table == 0 || table->handle == 0) {
        return;
    }
    column.mask = LVCF_TEXT | LVCF_WIDTH;
    column.cx = table->header_widths[column_index];
    column.pszText = table->headers[column_index];
    if (was_declared) {
        SendMessageW(table->handle, LVM_SETCOLUMNW,
                     (WPARAM)(unsigned __int64)column_index, (LPARAM)&column);
    } else {
        SendMessageW(table->handle, LVM_INSERTCOLUMNW,
                     (WPARAM)(unsigned __int64)column_index, (LPARAM)&column);
    }
}

void ui_table(int event_id, int x, int y, int width, int height,
              UINT text_color, UINT background_color) {
    TableSpec *table;
    if (desktop.table_count >= 4) {
        return;
    }
    table = &desktop.tables[desktop.table_count];
    desktop.table_count += 1;
    table->column_count = 0;
    table->row_count = 0;
    table->selected_row = -1;
    table->x = x;
    table->y = y;
    table->width = clamp_dimension(width, 1, 1600);
    table->height = clamp_dimension(height, 1, 1000);
    table->text_color = text_color;
    table->background_color = background_color;
    table->corner_radius = 0;
    table->event_id = event_id;
    table->id = TABLE_ID_BASE + (UINT)desktop.table_count;
    table->app_table_id = -1;
    table->app_table_column_count = 0;
    table->layout_managed = 0;
    table->updating = 0;
    table->handle = 0;
    table->brush = 0;
}

void ui_table_column(int event_id, int column_index,
                     const char *title_data, unsigned __int64 title_length,
                     int width) {
    TableSpec *table = table_for_event_id(event_id);
    int was_declared;
    if (table == 0 || column_index < 0 || column_index >= 8) {
        return;
    }
    was_declared = column_index < table->column_count;
    copy_utf8((JadrenString){title_data, (LONGLONG)title_length},
              table->headers[column_index], 160);
    table->header_widths[column_index] = clamp_dimension(width, 40, 800);
    if (column_index >= table->column_count) {
        table->column_count = column_index + 1;
    }
    update_table_column(table, column_index, was_declared);
    refresh_table_control(table);
}

void ui_table_cell(int event_id, int row_index, int column_index,
                   const char *text_data, unsigned __int64 text_length) {
    TableSpec *table = table_for_event_id(event_id);
    if (table == 0 || row_index < 0 || row_index >= 64 || column_index < 0 ||
        column_index >= table->column_count) {
        return;
    }
    copy_utf8((JadrenString){text_data, (LONGLONG)text_length},
              table->cells[row_index][column_index], 160);
    if (row_index >= table->row_count) {
        table->row_count = row_index + 1;
    }
    refresh_table_control(table);
    sync_state_bindings_for_event(event_id);
}

/* Pull a bounded application table into the native report view. The caller
 * supplies the visible column count because app_table is intentionally a
 * schema-free text matrix. Empty cells are copied as empty strings. */
static void refresh_table_from_app(TableSpec *table) {
#if JADREN_UI_HAS_FILE_RUNTIME
    unsigned char output[256];
    int row_index;
    int column_index;
    int rows;
    if (table == 0 || table->app_table_id < 0 ||
        table->app_table_column_count <= 0) {
        return;
    }
    rows = app_table_row_count(table->app_table_id);
    if (rows < 0) rows = 0;
    if (rows > 64) rows = 64;
    table->column_count = table->app_table_column_count;
    table->row_count = rows;
    if (table->selected_row >= rows) table->selected_row = -1;
    for (row_index = 0; row_index < rows; row_index += 1) {
        for (column_index = 0; column_index < table->column_count; column_index += 1) {
            unsigned __int64 copied = app_table_read_cell(
                table->app_table_id, row_index, column_index,
                output, sizeof(output));
            copy_utf8((JadrenString){(const char *)output, (LONGLONG)copied},
                      table->cells[row_index][column_index], 160);
        }
    }
    refresh_table_control(table);
    sync_state_bindings_for_event(table->event_id);
#else
    (void)table;
#endif
}

static void refresh_tables_bound_to_app(int app_table_id) {
    int index;
    for (index = 0; index < desktop.table_count; index += 1) {
        if (desktop.tables[index].app_table_id == app_table_id) {
            refresh_table_from_app(&desktop.tables[index]);
        }
    }
}

static int sort_table_from_app_kind(TableSpec *table, int column_index,
                                    int descending, int kind) {
#if JADREN_UI_HAS_FILE_RUNTIME
    int (*sort_function)(int, int, int);
    if (table == 0 || table->app_table_id < 0 || column_index < 0 ||
        column_index >= table->app_table_column_count) {
        return 0;
    }
    sort_function = app_table_sort_text;
    if (kind == 1) sort_function = app_table_sort_int;
    else if (kind == 2) sort_function = app_table_sort_uint;
    else if (kind == 3) sort_function = app_table_sort_float;
    else if (kind == 4) sort_function = app_table_sort_bool;
    else if (kind != 0) return 0;
    if (!sort_function(table->app_table_id, column_index, descending)) {
        return 0;
    }
    refresh_tables_bound_to_app(table->app_table_id);
    return 1;
#else
    (void)table;
    (void)column_index;
    (void)descending;
    return 0;
#endif
}

static int sort_table_from_app(TableSpec *table, int column_index, int descending) {
    return sort_table_from_app_kind(table, column_index, descending, 0);
}

static int filter_table_from_app(TableSpec *table, int destination_table_id,
                                 int column_index, const char *query_data,
                                 unsigned __int64 query_length, int mode) {
#if JADREN_UI_HAS_FILE_RUNTIME
    int result;
    if (table == 0 || table->app_table_id < 0 || destination_table_id < 0 ||
        destination_table_id == table->app_table_id || column_index < 0 ||
        column_index >= table->app_table_column_count ||
        (query_data == 0 && query_length > 0) || mode < 0 || mode > 7) {
        return 0;
    }
    result = app_table_filter_text_ex(table->app_table_id, destination_table_id,
                                      column_index, query_data, query_length, mode);
    if (!result) {
        return 0;
    }
    refresh_tables_bound_to_app(destination_table_id);
    return 1;
#else
    (void)table;
    (void)destination_table_id;
    (void)column_index;
    (void)query_data;
    (void)query_length;
    (void)mode;
    return 0;
#endif
}

static int filter_table_from_app_int(TableSpec *table, int destination_table_id,
                                     int column_index, long long query) {
#if JADREN_UI_HAS_FILE_RUNTIME
    int result;
    if (table == 0 || table->app_table_id < 0 || destination_table_id < 0 ||
        destination_table_id == table->app_table_id || column_index < 0 ||
        column_index >= table->app_table_column_count) return 0;
    result = app_table_filter_int(table->app_table_id, destination_table_id,
                                  column_index, query);
    if (!result) return 0;
    refresh_tables_bound_to_app(destination_table_id);
    return 1;
#else
    (void)table; (void)destination_table_id; (void)column_index; (void)query;
    return 0;
#endif
}

static int filter_table_from_app_uint(TableSpec *table, int destination_table_id,
                                      int column_index, unsigned __int64 query) {
#if JADREN_UI_HAS_FILE_RUNTIME
    int result;
    if (table == 0 || table->app_table_id < 0 || destination_table_id < 0 ||
        destination_table_id == table->app_table_id || column_index < 0 ||
        column_index >= table->app_table_column_count) return 0;
    result = app_table_filter_uint(table->app_table_id, destination_table_id,
                                   column_index, query);
    if (!result) return 0;
    refresh_tables_bound_to_app(destination_table_id);
    return 1;
#else
    (void)table; (void)destination_table_id; (void)column_index; (void)query;
    return 0;
#endif
}

static int filter_table_from_app_float(TableSpec *table, int destination_table_id,
                                       int column_index, double query) {
#if JADREN_UI_HAS_FILE_RUNTIME
    int result;
    if (table == 0 || table->app_table_id < 0 || destination_table_id < 0 ||
        destination_table_id == table->app_table_id || column_index < 0 ||
        column_index >= table->app_table_column_count) return 0;
    result = app_table_filter_float(table->app_table_id, destination_table_id,
                                    column_index, query);
    if (!result) return 0;
    refresh_tables_bound_to_app(destination_table_id);
    return 1;
#else
    (void)table; (void)destination_table_id; (void)column_index; (void)query;
    return 0;
#endif
}

static int filter_table_from_app_bool(TableSpec *table, int destination_table_id,
                                      int column_index, unsigned char query) {
#if JADREN_UI_HAS_FILE_RUNTIME
    int result;
    if (table == 0 || table->app_table_id < 0 || destination_table_id < 0 ||
        destination_table_id == table->app_table_id || column_index < 0 ||
        column_index >= table->app_table_column_count) return 0;
    result = app_table_filter_bool(table->app_table_id, destination_table_id,
                                   column_index, query);
    if (!result) return 0;
    refresh_tables_bound_to_app(destination_table_id);
    return 1;
#else
    (void)table; (void)destination_table_id; (void)column_index; (void)query;
    return 0;
#endif
}

void ui_table_sort_text(int event_id, int column_index, int descending) {
    (void)sort_table_from_app(table_for_event_id(event_id), column_index, descending);
}

void ui_table_sort_int(int event_id, int column_index, int descending) {
    (void)sort_table_from_app_kind(table_for_event_id(event_id), column_index,
                                   descending, 1);
}

void ui_table_sort_uint(int event_id, int column_index, int descending) {
    (void)sort_table_from_app_kind(table_for_event_id(event_id), column_index,
                                   descending, 2);
}

void ui_table_sort_float(int event_id, int column_index, int descending) {
    (void)sort_table_from_app_kind(table_for_event_id(event_id), column_index,
                                   descending, 3);
}

void ui_table_sort_bool(int event_id, int column_index, int descending) {
    (void)sort_table_from_app_kind(table_for_event_id(event_id), column_index,
                                   descending, 4);
}

void ui_table_filter_text(int event_id, int destination_table_id, int column_index,
                          const char *query_data, unsigned __int64 query_length) {
    (void)filter_table_from_app(table_for_event_id(event_id), destination_table_id,
                                column_index, query_data, query_length, 0);
}

void ui_table_filter_text_ex(int event_id, int destination_table_id, int column_index,
                             const char *query_data, unsigned __int64 query_length,
                             int mode) {
    (void)filter_table_from_app(table_for_event_id(event_id), destination_table_id,
                                column_index, query_data, query_length, mode);
}

void ui_table_filter_int(int event_id, int destination_table_id, int column_index,
                         long long query) {
    (void)filter_table_from_app_int(table_for_event_id(event_id), destination_table_id,
                                    column_index, query);
}

void ui_table_filter_uint(int event_id, int destination_table_id, int column_index,
                          unsigned __int64 query) {
    (void)filter_table_from_app_uint(table_for_event_id(event_id), destination_table_id,
                                     column_index, query);
}

void ui_table_filter_float(int event_id, int destination_table_id, int column_index,
                           double query) {
    (void)filter_table_from_app_float(table_for_event_id(event_id), destination_table_id,
                                      column_index, query);
}

void ui_table_filter_bool(int event_id, int destination_table_id, int column_index,
                          unsigned char query) {
    (void)filter_table_from_app_bool(table_for_event_id(event_id), destination_table_id,
                                     column_index, query);
}

void ui_table_bind_app(int event_id, int table_id, int column_count) {
    TableSpec *table = table_for_event_id(event_id);
    if (table == 0 || table_id < 0 || column_count < 1 || column_count > 8) {
        return;
    }
    table->app_table_id = table_id;
    table->app_table_column_count = column_count;
    refresh_table_from_app(table);
}

void ui_table_refresh_app(int event_id) {
    refresh_table_from_app(table_for_event_id(event_id));
}

/* Refresh every bound native list, table and text input in one deterministic
 * pass. This is the natural update boundary for an application callback: the
 * Jadren model stays the source of truth, while controls never render a stale
 * projection after a mutation. */
void ui_refresh_bindings(void) {
#if JADREN_UI_HAS_FILE_RUNTIME
    int index;
    for (index = 0; index < desktop.input_count; index += 1) {
        if (input_app_binding_for_event_id(desktop.inputs[index].event_id) != 0) {
            refresh_input_from_app_state(&desktop.inputs[index]);
        }
    }
    for (index = 0; index < desktop.button_count; index += 1) {
        ButtonSpec *button = &desktop.buttons[index];
        InputAppBinding *binding = input_app_binding_for_event_id(button->event_id);
        if (binding != 0 && binding->kind == JADREN_APP_BIND_BOOL &&
            (button->is_checkbox || button->is_switch)) {
            refresh_checkbox_from_app_state(button);
        }
    }
    for (index = 0; index < desktop.select_count; index += 1) {
        SelectSpec *select = &desktop.selects[index];
        InputAppBinding *binding = input_app_binding_for_event_id(select->event_id);
        if (binding != 0 && binding->kind == JADREN_APP_BIND_SELECT) {
            refresh_select_from_app_state(select);
        }
    }
    for (index = 0; index < desktop.list_count; index += 1) {
        refresh_list_from_app(&desktop.lists[index]);
        refresh_list_from_app_state(&desktop.lists[index]);
    }
    for (index = 0; index < desktop.table_count; index += 1) {
        refresh_table_from_app(&desktop.tables[index]);
        refresh_table_from_app_state(&desktop.tables[index]);
    }
#endif
}

static void refresh_app_bindings(void) {
    ui_refresh_bindings();
}

void ui_table_clear(int event_id) {
    TableSpec *table = table_for_event_id(event_id);
    if (table == 0) {
        return;
    }
    table->row_count = 0;
    table->selected_row = -1;
    sync_table_app_state(table);
    refresh_table_control(table);
    sync_state_bindings_for_event(event_id);
}

int ui_table_row_count(int event_id) {
    TableSpec *table = table_for_event_id(event_id);
    return table == 0 ? 0 : table->row_count;
}

unsigned __int64 ui_table_read_cell(int event_id, int row_index, int column_index,
                                    unsigned char *output_data,
                                    unsigned __int64 output_length) {
    TableSpec *table = table_for_event_id(event_id);
    if (table == 0 || row_index < 0 || row_index >= table->row_count ||
        column_index < 0 || column_index >= table->column_count) {
        return 0;
    }
    return copy_wide_utf8(table->cells[row_index][column_index],
                          output_data, output_length);
}

int ui_table_selected_row(int event_id) {
    TableSpec *table = table_for_event_id(event_id);
    return table == 0 ? -1 : table->selected_row;
}

void ui_table_set_selected_row(int event_id, int row_index) {
    TableSpec *table = table_for_event_id(event_id);
    LVITEMW state = {0};
    int was_updating;
    if (table == 0 || row_index < -1 || row_index >= table->row_count) {
        return;
    }
    table->selected_row = row_index;
    sync_table_app_state(table);
    sync_state_bindings_for_event(event_id);
    if (table->handle == 0) {
        return;
    }
    state.mask = LVIF_STATE;
    state.state = row_index >= 0 ? LVIS_SELECTED : 0;
    state.stateMask = LVIS_SELECTED;
    state.iItem = row_index;
    was_updating = table->updating;
    table->updating = 1;
    SendMessageW(table->handle, LVM_SETITEMSTATE,
                 (WPARAM)(unsigned __int64)(row_index >= 0 ? row_index : -1),
                 (LPARAM)&state);
    table->updating = was_updating;
}

static void sync_list_app_state(ListSpec *list) {
#if JADREN_UI_HAS_FILE_RUNTIME
    InputAppBinding *binding;
    if (list == 0) return;
    binding = input_app_binding_for_event_id(list->event_id);
    if (binding == 0 || binding->kind != JADREN_APP_BIND_LIST) return;
    (void)app_state_set_int(binding->key, binding->key_length,
                            (long long)list->selected_index);
#else
    (void)list;
#endif
}

static void refresh_list_from_app_state(ListSpec *list) {
#if JADREN_UI_HAS_FILE_RUNTIME
    InputAppBinding *binding;
    long long value;
    if (list == 0) return;
    binding = input_app_binding_for_event_id(list->event_id);
    if (binding == 0 || binding->kind != JADREN_APP_BIND_LIST) return;
    value = app_state_get_int(binding->key, binding->key_length);
    if (value < -1 || value >= (long long)list->item_count) value = -1;
    list->selected_index = (int)value;
    if (list->handle != 0) {
        SendMessageW(list->handle, LB_SETCURSEL,
                     (WPARAM)(unsigned __int64)(value < 0 ? -1 : value), 0);
    }
    sync_state_bindings_for_event(list->event_id);
#else
    (void)list;
#endif
}

static void sync_table_app_state(TableSpec *table) {
#if JADREN_UI_HAS_FILE_RUNTIME
    InputAppBinding *binding;
    if (table == 0) return;
    binding = input_app_binding_for_event_id(table->event_id);
    if (binding == 0 || binding->kind != JADREN_APP_BIND_TABLE) return;
    (void)app_state_set_int(binding->key, binding->key_length,
                            (long long)table->selected_row);
#else
    (void)table;
#endif
}

/* A callback can originate from a different control than the one whose value
 * changed (for example a Save button after a programmatic WM_SETTEXT). Push
 * every bound native projection into app_state before user code runs so the
 * model is always the source of truth at the event boundary. */
static void sync_app_bindings_from_native(void) {
#if JADREN_UI_HAS_FILE_RUNTIME
    int index;
    for (index = 0; index < desktop.input_count; index += 1) {
        InputSpec *input = &desktop.inputs[index];
        if (input_app_binding_for_event_id(input->event_id) != 0) {
            sync_input_app_state(input);
            sync_state_bindings_for_event(input->event_id);
        }
    }
    for (index = 0; index < desktop.button_count; index += 1) {
        ButtonSpec *button = &desktop.buttons[index];
        InputAppBinding *binding =
            input_app_binding_for_event_id(button->event_id);
        if (binding != 0 && binding->kind == JADREN_APP_BIND_BOOL &&
            (button->is_checkbox || button->is_switch)) {
            sync_checkbox_app_state(button);
            sync_state_bindings_for_event(button->event_id);
        }
    }
    for (index = 0; index < desktop.select_count; index += 1) {
        SelectSpec *select = &desktop.selects[index];
        if (input_app_binding_for_event_id(select->event_id) != 0) {
            if (select->handle != 0) {
                select->selected_index = (int)SendMessageW(
                    select->handle, CB_GETCURSEL, 0, 0);
            }
            sync_select_app_state(select);
            sync_state_bindings_for_event(select->event_id);
        }
    }
    for (index = 0; index < desktop.list_count; index += 1) {
        ListSpec *list = &desktop.lists[index];
        if (input_app_binding_for_event_id(list->event_id) != 0) {
            if (list->handle != 0) {
                list->selected_index = (int)SendMessageW(
                    list->handle, LB_GETCURSEL, 0, 0);
            }
            sync_list_app_state(list);
            sync_state_bindings_for_event(list->event_id);
        }
    }
    for (index = 0; index < desktop.table_count; index += 1) {
        TableSpec *table = &desktop.tables[index];
        if (input_app_binding_for_event_id(table->event_id) != 0) {
            sync_table_app_state(table);
            sync_state_bindings_for_event(table->event_id);
        }
    }
#endif
}

static void refresh_table_from_app_state(TableSpec *table) {
#if JADREN_UI_HAS_FILE_RUNTIME
    InputAppBinding *binding;
    long long value;
    LVITEMW state = {0};
    int was_updating;
    if (table == 0) return;
    binding = input_app_binding_for_event_id(table->event_id);
    if (binding == 0 || binding->kind != JADREN_APP_BIND_TABLE) return;
    value = app_state_get_int(binding->key, binding->key_length);
    if (value < -1 || value >= (long long)table->row_count) value = -1;
    table->selected_row = (int)value;
    if (table->handle != 0) {
        /* Applying model state is a projection update, not user input. Keep
         * the native notification from re-entering the Jadren callback while
         * the ListView changes its selection. */
        was_updating = table->updating;
        table->updating = 1;
        state.mask = LVIF_STATE;
        state.state = value >= 0 ? LVIS_SELECTED : 0;
        state.stateMask = LVIS_SELECTED;
        state.iItem = (int)value;
        SendMessageW(table->handle, LVM_SETITEMSTATE,
                     (WPARAM)(unsigned __int64)(value >= 0 ? value : -1),
                     (LPARAM)&state);
        table->updating = was_updating;
    }
    sync_state_bindings_for_event(table->event_id);
#else
    (void)table;
#endif
}

int ui_list_bind_app_state(int event_id, const char *key_data,
                           unsigned __int64 key_length) {
    InputAppBinding *binding = input_app_binding_for_event_id(event_id);
    ListSpec *list = list_for_event_id(event_id);
    if (list == 0 || key_data == 0 || key_length == 0 || key_length > 64) {
        return 0;
    }
    if (binding == 0) {
        if (desktop.input_app_binding_count >= 12) return 0;
        binding = &desktop.input_app_bindings[desktop.input_app_binding_count];
        desktop.input_app_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->kind = JADREN_APP_BIND_LIST;
    binding->key_length = key_length;
    copy_utf8_bytes(key_data, key_length, binding->key, 65);
#if JADREN_UI_HAS_FILE_RUNTIME
    if (!portable_ui_preserve_app_binding &&
        !app_state_set_int(binding->key, binding->key_length,
                           (long long)list->selected_index)) {
        return 0;
    }
#endif
    if (!portable_ui_preserve_app_binding) {
        sync_list_app_state(list);
    }
    return 1;
}

void ui_list_refresh_app_state(int event_id) {
    refresh_list_from_app_state(list_for_event_id(event_id));
}

int ui_table_bind_app_state(int event_id, const char *key_data,
                            unsigned __int64 key_length) {
    InputAppBinding *binding = input_app_binding_for_event_id(event_id);
    TableSpec *table = table_for_event_id(event_id);
    if (table == 0 || key_data == 0 || key_length == 0 || key_length > 64) {
        return 0;
    }
    if (binding == 0) {
        if (desktop.input_app_binding_count >= 12) return 0;
        binding = &desktop.input_app_bindings[desktop.input_app_binding_count];
        desktop.input_app_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->kind = JADREN_APP_BIND_TABLE;
    binding->key_length = key_length;
    copy_utf8_bytes(key_data, key_length, binding->key, 65);
#if JADREN_UI_HAS_FILE_RUNTIME
    if (!portable_ui_preserve_app_binding &&
        !app_state_set_int(binding->key, binding->key_length,
                           (long long)table->selected_row)) {
        return 0;
    }
#endif
    if (!portable_ui_preserve_app_binding) {
        sync_table_app_state(table);
    }
    return 1;
}

void ui_table_refresh_app_state(int event_id) {
    refresh_table_from_app_state(table_for_event_id(event_id));
}

/*
 * Responsive layout builder. A root `ui_column` owns nested rows and panels.
 * Each child keeps an explicit width/height, while `stretch` fills the cross
 * axis. `align` (0 start, 1 centre, 2 end) positions non-stretched children
 * horizontally in columns and the complete group horizontally in rows. This
 * keeps the preview deterministic without a hidden flexbox.
 */
static int active_layout(void) {
    if (desktop.layout_depth <= 0) {
        return -1;
    }
    return desktop.layout_stack[desktop.layout_depth - 1];
}

static int normalize_layout_align(int align) {
    if (align < UI_ALIGN_START || align > UI_ALIGN_END) {
        return UI_ALIGN_START;
    }
    return align;
}

static int add_layout_node(int parent, int orientation, int x, int y,
                           int width, int height, int padding, int gap,
                           int align, int stretch) {
    LayoutNode *node;
    int node_index;
    if (desktop.layout_count >= 16 || desktop.layout_depth >= 16) {
        return -1;
    }
    node_index = desktop.layout_count;
    desktop.layout_count += 1;
    node = &desktop.layouts[node_index];
    node->parent = parent;
    node->orientation = orientation;
    node->x = x;
    node->y = y;
    node->width = clamp_dimension(width, 1, 4000);
    node->height = clamp_dimension(height, 1, 4000);
    node->base_width = node->width;
    node->base_height = node->height;
    node->padding = clamp_dimension(padding, 0, 128);
    node->gap = clamp_dimension(gap, 0, 128);
    node->align = normalize_layout_align(align);
    node->stretch = stretch != 0;
    node->panel_label = -1;
    node->child_count = 0;
    return node_index;
}

static void add_layout_child(int parent, int kind, int index,
                             int width, int height, int stretch) {
    LayoutNode *node;
    LayoutChild *child;
    if (parent < 0 || parent >= desktop.layout_count) {
        return;
    }
    node = &desktop.layouts[parent];
    if (node->child_count >= 24) {
        return;
    }
    child = &node->children[node->child_count];
    node->child_count += 1;
    child->kind = kind;
    child->index = index;
    child->width = clamp_dimension(width, 1, 4000);
    child->height = clamp_dimension(height, 1, 4000);
    child->stretch = stretch != 0;
}

void ui_column(int x, int y, int width, int height,
               int padding, int gap, int align, int stretch) {
    int node;
    if (desktop.layout_depth != 0) {
        return;
    }
    node = add_layout_node(-1, LAYOUT_COLUMN, x, y, width, height,
                           padding, gap, align, stretch);
    if (node >= 0) {
        desktop.layout_stack[desktop.layout_depth] = node;
        desktop.layout_depth += 1;
    }
}

void ui_row(int width, int height, int padding, int gap, int align, int stretch) {
    int parent = active_layout();
    int node;
    if (parent < 0) {
        return;
    }
    node = add_layout_node(parent, LAYOUT_ROW, 0, 0, width, height,
                           padding, gap, align, stretch);
    if (node >= 0) {
        add_layout_child(parent, LAYOUT_CHILD_NODE, node, width, height, stretch);
        desktop.layout_stack[desktop.layout_depth] = node;
        desktop.layout_depth += 1;
    }
}

void ui_panel(int width, int height, UINT background_color, int corner_radius,
              int padding, int gap, int align, int stretch) {
    int parent = active_layout();
    int node;
    LabelSpec *panel;
    int panel_index;
    if (parent < 0) {
        return;
    }
    panel_index = desktop.label_count;
    panel = add_label((JadrenString){0, 0}, 0, 0, width, height,
                      0, background_color, corner_radius, 0);
    node = add_layout_node(parent, LAYOUT_COLUMN, 0, 0, width, height,
                           padding, gap, align, stretch);
    if (node < 0) {
        return;
    }
    if (panel != 0) {
        panel->layout_managed = 1;
        desktop.layouts[node].panel_label = panel_index;
    }
    add_layout_child(parent, LAYOUT_CHILD_NODE, node, width, height, stretch);
    desktop.layout_stack[desktop.layout_depth] = node;
    desktop.layout_depth += 1;
}

/* A retained top bar is a horizontal layout scope with an explicit native
 * background label behind its children. Child controls remain ordinary
 * layout-managed buttons/labels, so resize and redraw use the same pass as a
 * regular panel. */
void ui_layout_top_bar(int width, int height, UINT background_color,
                       int corner_radius, int padding, int gap,
                       int align, int stretch) {
    int parent = active_layout();
    int node;
    int bar_index;
    LabelSpec *bar;
    if (parent < 0) {
        return;
    }
    bar_index = desktop.label_count;
    bar = add_label((JadrenString){0, 0}, 0, 0, width, height,
                    0, background_color, corner_radius, 0);
    node = add_layout_node(parent, LAYOUT_ROW, 0, 0, width, height,
                           padding, gap, align, stretch);
    if (node < 0) {
        return;
    }
    if (bar != 0) {
        bar->layout_managed = 1;
        desktop.layouts[node].panel_label = bar_index;
    }
    add_layout_child(parent, LAYOUT_CHILD_NODE, node, width, height, stretch);
    desktop.layout_stack[desktop.layout_depth] = node;
    desktop.layout_depth += 1;
}

/* A retained menu trigger uses the same layout-managed button geometry as
 * ordinary retained controls, while its options are owned by a native popup
 * MenuSpec. The popup itself is created only after the trigger is clicked, so
 * resize never leaves a second child window or stale menu surface behind. */
static BOOL ui_layout_menu(const char *label_data, unsigned __int64 label_length,
                           int menu_id, int width, int height,
                           UINT text_color, UINT background_color,
                           int corner_radius, int stretch) {
    int parent = active_layout();
    int button_index;
    ButtonSpec *button;
    MenuSpec *menu;
    if (parent < 0 || parent >= desktop.layout_count ||
        desktop.layouts[parent].child_count >= 24 ||
        desktop.menu_count >= 8 || desktop.button_count >= 24) {
        return 0;
    }
    button_index = desktop.button_count;
    button = add_button(
        (JadrenString){label_data, (LONGLONG)label_length}, (JadrenString){0, 0},
        0, 0, width, height, text_color, background_color,
        corner_radius, 0, 1, 0, 0, 1, 0, 0);
    if (button == 0) {
        return 0;
    }
    button->layout_managed = 1;
    menu = &desktop.menus[desktop.menu_count];
    desktop.menu_count += 1;
    menu->option_count = 0;
    menu->menu_id = menu_id;
    menu->button_id = button->id;
    add_layout_child(parent, LAYOUT_CHILD_BUTTON, button_index,
                     width, height, stretch);
    return 1;
}

void ui_layout_end(void) {
    if (desktop.layout_depth > 0) {
        desktop.layout_depth -= 1;
    }
}

static void add_layout_label(JadrenString text, int width, int height,
                             UINT text_color, UINT background_color,
                             int corner_radius, int stretch, BOOL is_status) {
    int parent = active_layout();
    LabelSpec *label;
    int label_index;
    if (parent < 0) {
        return;
    }
    label_index = desktop.label_count;
    label = add_label(text, 0, 0, width, height, text_color, background_color,
                      corner_radius, 0);
    if (label == 0) {
        return;
    }
    label->layout_managed = 1;
    if (is_status) {
        desktop.status_index = label_index;
    }
    add_layout_child(parent, LAYOUT_CHILD_LABEL, label_index, width, height, stretch);
}

void ui_layout_label(const char *text_data, unsigned __int64 text_length,
                     int width, int height, UINT text_color,
                     UINT background_color, int corner_radius, int stretch) {
    add_layout_label((JadrenString){text_data, (LONGLONG)text_length},
                     width, height, text_color, background_color,
                     corner_radius, stretch, 0);
}

void ui_layout_status(const char *text_data, unsigned __int64 text_length,
                      int width, int height, UINT text_color,
                      UINT background_color, int corner_radius, int stretch) {
    add_layout_label((JadrenString){text_data, (LONGLONG)text_length},
                     width, height, text_color, background_color,
                     corner_radius, stretch, 1);
}

static void add_layout_text_input(const char *text_data,
                                   unsigned __int64 text_length,
                                   int event_id, int width, int height,
                                   UINT text_color, UINT background_color,
                                   int corner_radius, int stretch) {
    int parent = active_layout();
    InputSpec *input;
    int input_index;
    int before_inputs;
    if (parent < 0) {
        return;
    }
    before_inputs = desktop.input_count;
    ui_text_input(text_data, text_length, event_id, 0, 0, width, height,
                  text_color, background_color, corner_radius);
    if (desktop.input_count != before_inputs + 1) {
        return;
    }
    input_index = desktop.input_count - 1;
    input = &desktop.inputs[input_index];
    input->layout_managed = 1;
    add_layout_child(parent, LAYOUT_CHILD_INPUT, input_index, width, height, stretch);
}

static void add_layout_checkbox(const char *label_data,
                                unsigned __int64 label_length,
                                int event_id, int width, int height,
                                UINT text_color, UINT background_color,
                                int corner_radius, int stretch, int checked) {
    int parent = active_layout();
    ButtonSpec *button;
    int button_index;
    int before_buttons;
    if (parent < 0) {
        return;
    }
    before_buttons = desktop.button_count;
    ui_checkbox(label_data, label_length, event_id, 0, 0, width, height,
                text_color, background_color, corner_radius);
    if (desktop.button_count != before_buttons + 1) {
        return;
    }
    button_index = desktop.button_count - 1;
    button = &desktop.buttons[button_index];
    button->layout_managed = 1;
    button->active = checked != 0;
    add_layout_child(parent, LAYOUT_CHILD_BUTTON, button_index, width, height, stretch);
}

static void add_layout_select(int event_id, int width, int height,
                              UINT text_color, UINT background_color,
                              int corner_radius, int stretch) {
    int parent = active_layout();
    SelectSpec *select;
    int select_index;
    int before_selects;
    if (parent < 0) {
        return;
    }
    before_selects = desktop.select_count;
    ui_select(event_id, 0, 0, width, height, text_color, background_color);
    if (desktop.select_count != before_selects + 1) {
        return;
    }
    select_index = desktop.select_count - 1;
    select = &desktop.selects[select_index];
    select->corner_radius = clamp_dimension(corner_radius, 0, 128);
    select->layout_managed = 1;
    add_layout_child(parent, LAYOUT_CHILD_SELECT, select_index, width, height, stretch);
}

static void add_layout_list(int event_id, int width, int height,
                            UINT text_color, UINT background_color,
                            int corner_radius, int stretch) {
    int parent = active_layout();
    ListSpec *list;
    int list_index;
    int before_lists;
    if (parent < 0) {
        return;
    }
    before_lists = desktop.list_count;
    ui_list(event_id, 0, 0, width, height, text_color, background_color);
    if (desktop.list_count != before_lists + 1) {
        return;
    }
    list_index = desktop.list_count - 1;
    list = &desktop.lists[list_index];
    list->corner_radius = clamp_dimension(corner_radius, 0, 128);
    list->layout_managed = 1;
    add_layout_child(parent, LAYOUT_CHILD_LIST, list_index, width, height, stretch);
}

static void add_layout_table(int event_id, int width, int height,
                             UINT text_color, UINT background_color,
                             int corner_radius, int stretch) {
    int parent = active_layout();
    TableSpec *table;
    int table_index;
    int before_tables;
    if (parent < 0) {
        return;
    }
    before_tables = desktop.table_count;
    ui_table(event_id, 0, 0, width, height, text_color, background_color);
    if (desktop.table_count != before_tables + 1) {
        return;
    }
    table_index = desktop.table_count - 1;
    table = &desktop.tables[table_index];
    table->corner_radius = clamp_dimension(corner_radius, 0, 128);
    table->layout_managed = 1;
    add_layout_child(parent, LAYOUT_CHILD_TABLE, table_index, width, height, stretch);
}

void ui_layout_event_button(const char *label_data, unsigned __int64 label_length,
                            int event_id, int width, int height,
                            UINT text_color, UINT background_color,
                            int corner_radius, int stretch) {
    int parent = active_layout();
    ButtonSpec *button;
    int button_index;
    if (parent < 0) {
        return;
    }
    button_index = desktop.button_count;
    button = add_button(
        (JadrenString){label_data, (LONGLONG)label_length}, (JadrenString){0, 0},
        0, 0, width, height, text_color, background_color,
        corner_radius, 0, 0, 0, 0, 0, 0, 0);
    if (button == 0) {
        return;
    }
    button->event_id = event_id;
    button->emits_event = 1;
    button->layout_managed = 1;
    add_layout_child(parent, LAYOUT_CHILD_BUTTON, button_index, width, height, stretch);
}

/*
 * Target-neutral retained UI contract.
 *
 * These functions deliberately expose only integer node ids and declarative
 * parent/child relationships.  The current Windows preview maps them to the
 * layout builder above; a future Linux, macOS or embedded backend can keep
 * the same Jadren source and ABI without exposing HWNDs or Win32 coordinates.
 * `ui_app_text_input`, `ui_app_checkbox`, `ui_app_select`, `ui_app_list` and
 * `ui_app_table`
 * are retained value controls; their native EDIT/owner-drawn
 * BUTTON/COMBOBOX/LISTBOX controls participate in the same parent layout and
 * callback boundary. Select options and list mutations are addressed by
 * retained node id, then translated to the existing event-id based native
 * model.
 */
static int portable_ui_next_node;
static int portable_ui_stack[16];
static int portable_ui_depth;
static BOOL portable_ui_active;
static int portable_ui_event_ids[128];
static BOOL portable_ui_event_bound[128];
static int portable_ui_select_event_ids[128];
static BOOL portable_ui_select_bound[128];
static int portable_ui_list_event_ids[128];
static BOOL portable_ui_list_bound[128];
static int portable_ui_table_event_ids[128];
static BOOL portable_ui_table_bound[128];

static int portable_ui_parent_is_current(int parent) {
    if (!portable_ui_active || portable_ui_depth <= 0) {
        return 0;
    }
    return portable_ui_stack[portable_ui_depth - 1] == parent;
}

static int portable_ui_event_for_node(int node) {
    if (node <= 0 || node > 127 || !portable_ui_event_bound[node]) {
        return 0;
    }
    return portable_ui_event_ids[node];
}

static int portable_ui_bind_event_node(int node, int event_id) {
    if (node <= 0 || node > 127 || event_id == 0) {
        return 0;
    }
    portable_ui_event_ids[node] = event_id;
    portable_ui_event_bound[node] = 1;
    return node;
}

static int portable_ui_allocate_node(void) {
    int node;
    if (portable_ui_next_node > 127 || portable_ui_depth >= 16) {
        return 0;
    }
    node = portable_ui_next_node;
    portable_ui_next_node += 1;
    return node;
}

int ui_app_begin(const char *title_data, unsigned __int64 title_length,
                 int width, int height, UINT background_color) {
    int root;
    int before_depth;
    int index;
    if (portable_ui_active) {
        return 0;
    }
    portable_ui_next_node = 1;
    portable_ui_depth = 0;
    portable_ui_active = 0;
    for (index = 0; index < 128; index += 1) {
        portable_ui_event_ids[index] = 0;
        portable_ui_event_bound[index] = 0;
        portable_ui_select_event_ids[index] = 0;
        portable_ui_select_bound[index] = 0;
        portable_ui_list_event_ids[index] = 0;
        portable_ui_list_bound[index] = 0;
        portable_ui_table_event_ids[index] = 0;
        portable_ui_table_bound[index] = 0;
    }
    ui_window(title_data, title_length, width, height, background_color);
    before_depth = desktop.layout_depth;
    ui_column(16, 16, width - 32, height - 32, 16, 12, UI_ALIGN_START, 1);
    if (desktop.layout_depth != before_depth + 1) {
        return 0;
    }
    root = portable_ui_allocate_node();
    if (root == 0) {
        return 0;
    }
    portable_ui_stack[portable_ui_depth] = root;
    portable_ui_depth += 1;
    portable_ui_active = 1;
    return root;
}

int ui_app_panel(int parent, int width, int height, UINT background_color,
                 int corner_radius, int padding, int gap, int align, int stretch) {
    int before_depth;
    int before_layout_count;
    int node;
    if (!portable_ui_parent_is_current(parent)) {
        return 0;
    }
    before_depth = desktop.layout_depth;
    before_layout_count = desktop.layout_count;
    ui_panel(width, height, background_color, corner_radius, padding, gap, align, stretch);
    if (desktop.layout_depth != before_depth + 1 ||
        desktop.layout_count != before_layout_count + 1) {
        return 0;
    }
    node = portable_ui_allocate_node();
    if (node == 0) {
        ui_layout_end();
        return 0;
    }
    portable_ui_stack[portable_ui_depth] = node;
    portable_ui_depth += 1;
    return node;
}

/* Window lifecycle stays on the same explicit callback boundary as controls.
 * The native host owns the HWND; Jadren only registers event ids and reads
 * the current client geometry when its callback runs. */
void ui_app_on_resize(int event_id) {
    desktop.resize_event_id = event_id;
}

void ui_app_on_close(int event_id) {
    desktop.close_event_id = event_id;
}

int ui_app_window_width(void) {
    RECT client;
    if (desktop.window != 0 && GetClientRect(desktop.window, &client) != 0) {
        return (int)(client.right - client.left);
    }
    return desktop.width;
}

int ui_app_window_height(void) {
    RECT client;
    if (desktop.window != 0 && GetClientRect(desktop.window, &client) != 0) {
        return (int)(client.bottom - client.top);
    }
    return desktop.height;
}

BOOL ui_app_window_set_constraints(int min_width, int min_height,
                                   int max_width, int max_height) {
    if (min_width < 120 || min_height < 80 || max_width < min_width ||
        max_height < min_height || max_width > 8192 || max_height > 8192) {
        return 0;
    }
    desktop.min_width = min_width;
    desktop.min_height = min_height;
    desktop.max_width = max_width;
    desktop.max_height = max_height;
    return 1;
}

int ui_app_window_min_width(void) {
    return desktop.min_width;
}

int ui_app_window_min_height(void) {
    return desktop.min_height;
}

int ui_app_window_max_width(void) {
    return desktop.max_width;
}

int ui_app_window_max_height(void) {
    return desktop.max_height;
}

int ui_app_row(int parent, int width, int height, int padding, int gap,
               int align, int stretch) {
    int before_depth;
    int before_layout_count;
    int node;
    if (!portable_ui_parent_is_current(parent)) {
        return 0;
    }
    before_depth = desktop.layout_depth;
    before_layout_count = desktop.layout_count;
    ui_row(width, height, padding, gap, align, stretch);
    if (desktop.layout_depth != before_depth + 1 ||
        desktop.layout_count != before_layout_count + 1) {
        return 0;
    }
    node = portable_ui_allocate_node();
    if (node == 0) {
        ui_layout_end();
        return 0;
    }
    portable_ui_stack[portable_ui_depth] = node;
    portable_ui_depth += 1;
    return node;
}

int ui_app_top_bar(int parent, int width, int height, UINT background_color,
                   int corner_radius, int padding, int gap, int align,
                   int stretch) {
    int before_depth;
    int before_layout_count;
    int node;
    if (!portable_ui_parent_is_current(parent)) {
        return 0;
    }
    before_depth = desktop.layout_depth;
    before_layout_count = desktop.layout_count;
    ui_layout_top_bar(width, height, background_color, corner_radius,
                      padding, gap, align, stretch);
    if (desktop.layout_depth != before_depth + 1 ||
        desktop.layout_count != before_layout_count + 1) {
        return 0;
    }
    node = portable_ui_allocate_node();
    if (node == 0) {
        ui_layout_end();
        return 0;
    }
    portable_ui_stack[portable_ui_depth] = node;
    portable_ui_depth += 1;
    return node;
}

int ui_app_menu(int parent, const char *label_data,
                unsigned __int64 label_length, int width, int height,
                UINT text_color, UINT background_color, int corner_radius,
                int stretch) {
    int node;
    if (!portable_ui_parent_is_current(parent) || portable_ui_next_node > 127) {
        return 0;
    }
    node = portable_ui_next_node;
    if (!ui_layout_menu(label_data, label_length, node, width, height,
                        text_color, background_color, corner_radius, stretch)) {
        return 0;
    }
    portable_ui_next_node += 1;
    return node;
}

BOOL ui_app_menu_item(int menu_node, const char *label_data,
                      unsigned __int64 label_length, int event_id) {
    MenuSpec *menu = menu_for_id(menu_node);
    int before_options;
    if (menu == 0) {
        return 0;
    }
    before_options = menu->option_count;
    ui_menu_option(menu_node, label_data, label_length, event_id);
    return menu->option_count == before_options + 1;
}

int ui_app_label(int parent, const char *text_data, unsigned __int64 text_length,
                 int width, int height, UINT text_color, UINT background_color,
                 int corner_radius, int stretch) {
    int before_labels;
    if (!portable_ui_parent_is_current(parent)) {
        return 0;
    }
    before_labels = desktop.label_count;
    ui_layout_label(text_data, text_length, width, height, text_color,
                    background_color, corner_radius, stretch);
    if (desktop.label_count != before_labels + 1) {
        return 0;
    }
    return portable_ui_allocate_node();
}

int ui_app_status(int parent, const char *text_data, unsigned __int64 text_length,
                  int width, int height, UINT text_color, UINT background_color,
                  int corner_radius, int stretch) {
    int before_labels;
    if (!portable_ui_parent_is_current(parent)) {
        return 0;
    }
    before_labels = desktop.label_count;
    add_layout_label((JadrenString){text_data, (LONGLONG)text_length},
                     width, height, text_color, background_color,
                     corner_radius, stretch, 1);
    if (desktop.label_count != before_labels + 1) {
        return 0;
    }
    return portable_ui_allocate_node();
}

int ui_app_text_input(int parent, const char *text_data,
                      unsigned __int64 text_length, int event_id,
                      int width, int height, UINT text_color,
                      UINT background_color, int corner_radius, int stretch) {
    int before_inputs;
    int node;
    if (!portable_ui_parent_is_current(parent)) {
        return 0;
    }
    before_inputs = desktop.input_count;
    add_layout_text_input(text_data, text_length, event_id, width, height,
                          text_color, background_color, corner_radius, stretch);
    if (desktop.input_count != before_inputs + 1) {
        return 0;
    }
    node = portable_ui_allocate_node();
    return portable_ui_bind_event_node(node, event_id);
}

static int portable_ui_select_event_for_node(int select_node) {
    if (select_node <= 0 || select_node > 127 ||
        !portable_ui_select_bound[select_node]) {
        return 0;
    }
    return portable_ui_select_event_ids[select_node];
}

int ui_app_select(int parent, int event_id, int width, int height,
                  UINT text_color, UINT background_color,
                  int corner_radius, int stretch) {
    int before_selects;
    int node;
    if (!portable_ui_parent_is_current(parent)) {
        return 0;
    }
    before_selects = desktop.select_count;
    add_layout_select(event_id, width, height, text_color, background_color,
                      corner_radius, stretch);
    if (desktop.select_count != before_selects + 1) {
        return 0;
    }
    node = portable_ui_allocate_node();
    if (node == 0) {
        return 0;
    }
    portable_ui_select_event_ids[node] = event_id;
    portable_ui_select_bound[node] = 1;
    return node;
}

BOOL ui_app_select_option(int select_node, const char *text_data,
                          unsigned __int64 text_length) {
    int event_id = portable_ui_select_event_for_node(select_node);
    SelectSpec *select = select_for_event_id(event_id);
    int before_options;
    if (select == 0) {
        return 0;
    }
    before_options = select->option_count;
    ui_select_option(event_id, text_data, text_length);
    return select->option_count == before_options + 1;
}

int ui_app_select_index(int select_node) {
    int event_id = portable_ui_select_event_for_node(select_node);
    return ui_select_index(event_id);
}

BOOL ui_app_select_set_index(int select_node, int selected_index) {
    int event_id = portable_ui_select_event_for_node(select_node);
    SelectSpec *select = select_for_event_id(event_id);
    if (select == 0 || selected_index < 0 || selected_index >= select->option_count) {
        return 0;
    }
    ui_select_set_index(event_id, selected_index);
    return ui_select_index(event_id) == selected_index;
}

static int portable_ui_list_event_for_node(int list_node) {
    if (list_node <= 0 || list_node > 127 ||
        !portable_ui_list_bound[list_node]) {
        return 0;
    }
    return portable_ui_list_event_ids[list_node];
}

int ui_app_list(int parent, int event_id, int width, int height,
                UINT text_color, UINT background_color,
                int corner_radius, int stretch) {
    int before_lists;
    int node;
    if (!portable_ui_parent_is_current(parent)) {
        return 0;
    }
    before_lists = desktop.list_count;
    add_layout_list(event_id, width, height, text_color, background_color,
                    corner_radius, stretch);
    if (desktop.list_count != before_lists + 1) {
        return 0;
    }
    node = portable_ui_allocate_node();
    if (node == 0) {
        return 0;
    }
    portable_ui_list_event_ids[node] = event_id;
    portable_ui_list_bound[node] = 1;
    return node;
}

BOOL ui_app_list_item(int list_node, const char *text_data,
                      unsigned __int64 text_length) {
    int event_id = portable_ui_list_event_for_node(list_node);
    ListSpec *list = list_for_event_id(event_id);
    int before_items;
    if (list == 0) {
        return 0;
    }
    before_items = list->item_count;
    ui_list_item(event_id, text_data, text_length);
    return list->item_count == before_items + 1;
}

BOOL ui_app_list_clear(int list_node) {
    int event_id = portable_ui_list_event_for_node(list_node);
    ListSpec *list = list_for_event_id(event_id);
    if (list == 0) {
        return 0;
    }
    ui_list_clear(event_id);
    return list->item_count == 0 && list->selected_index == -1;
}

int ui_app_list_count(int list_node) {
    return ui_list_count(portable_ui_list_event_for_node(list_node));
}

unsigned __int64 ui_app_list_read_item(int list_node, int item_index,
                                       unsigned char *output_data,
                                       unsigned __int64 output_length) {
    return ui_list_read_item(portable_ui_list_event_for_node(list_node),
                             item_index, output_data, output_length);
}

int ui_app_list_index(int list_node) {
    return ui_list_index(portable_ui_list_event_for_node(list_node));
}

BOOL ui_app_list_set_index(int list_node, int selected_index) {
    int event_id = portable_ui_list_event_for_node(list_node);
    ListSpec *list = list_for_event_id(event_id);
    if (list == 0 || selected_index < 0 || selected_index >= list->item_count) {
        return 0;
    }
    ui_list_set_index(event_id, selected_index);
    return ui_list_index(event_id) == selected_index;
}

BOOL ui_app_list_bind_app(int list_node, int list_id) {
    int event_id = portable_ui_list_event_for_node(list_node);
    ListSpec *list = list_for_event_id(event_id);
    if (list == 0 || list_id < 0) {
        return 0;
    }
    ui_list_bind_app(event_id, list_id);
    return list->app_list_id == list_id;
}

BOOL ui_app_list_refresh(int list_node) {
    int event_id = portable_ui_list_event_for_node(list_node);
    ListSpec *list = list_for_event_id(event_id);
    if (list == 0) {
        return 0;
    }
    ui_list_refresh_app(event_id);
    return 1;
}

static int portable_ui_table_event_for_node(int table_node) {
    if (table_node <= 0 || table_node > 127 ||
        !portable_ui_table_bound[table_node]) {
        return 0;
    }
    return portable_ui_table_event_ids[table_node];
}

int ui_app_table(int parent, int event_id, int width, int height,
                 UINT text_color, UINT background_color,
                 int corner_radius, int stretch) {
    int before_tables;
    int node;
    if (!portable_ui_parent_is_current(parent)) {
        return 0;
    }
    before_tables = desktop.table_count;
    add_layout_table(event_id, width, height, text_color, background_color,
                     corner_radius, stretch);
    if (desktop.table_count != before_tables + 1) {
        return 0;
    }
    node = portable_ui_allocate_node();
    if (node == 0) {
        return 0;
    }
    portable_ui_table_event_ids[node] = event_id;
    portable_ui_table_bound[node] = 1;
    return node;
}

BOOL ui_app_table_column(int table_node, int column_index,
                         const char *title_data, unsigned __int64 title_length,
                         int width) {
    int event_id = portable_ui_table_event_for_node(table_node);
    TableSpec *table = table_for_event_id(event_id);
    int valid_width = clamp_dimension(width, 40, 800);
    if (table == 0 || column_index < 0 || column_index >= 8 ||
        title_data == 0 || title_length > 159 || valid_width < 40) {
        return 0;
    }
    ui_table_column(event_id, column_index, title_data, title_length, width);
    return column_index < table->column_count &&
           table->header_widths[column_index] == valid_width;
}

BOOL ui_app_table_cell(int table_node, int row_index, int column_index,
                       const char *text_data, unsigned __int64 text_length) {
    int event_id = portable_ui_table_event_for_node(table_node);
    TableSpec *table = table_for_event_id(event_id);
    if (table == 0 || row_index < 0 || row_index >= 64 ||
        column_index < 0 || column_index >= table->column_count ||
        text_data == 0 || text_length > 159) {
        return 0;
    }
    ui_table_cell(event_id, row_index, column_index, text_data, text_length);
    return 1;
}

unsigned __int64 ui_app_table_read_cell(int table_node, int row_index,
                                        int column_index,
                                        unsigned char *output_data,
                                        unsigned __int64 output_length) {
    return ui_table_read_cell(portable_ui_table_event_for_node(table_node),
                              row_index, column_index, output_data, output_length);
}

BOOL ui_app_table_bind_app(int table_node, int table_id, int column_count) {
    int event_id = portable_ui_table_event_for_node(table_node);
    TableSpec *table = table_for_event_id(event_id);
    if (table == 0 || table_id < 0 || column_count < 1 || column_count > 8) {
        return 0;
    }
    ui_table_bind_app(event_id, table_id, column_count);
    return table->app_table_id == table_id &&
           table->app_table_column_count == column_count;
}

BOOL ui_app_table_sort_text(int table_node, int column_index, int descending) {
    return sort_table_from_app(table_for_event_id(
                                   portable_ui_table_event_for_node(table_node)),
                               column_index, descending);
}

BOOL ui_app_table_sort_int(int table_node, int column_index, int descending) {
    return sort_table_from_app_kind(table_for_event_id(
                                       portable_ui_table_event_for_node(table_node)),
                                   column_index, descending, 1);
}

BOOL ui_app_table_sort_uint(int table_node, int column_index, int descending) {
    return sort_table_from_app_kind(table_for_event_id(
                                       portable_ui_table_event_for_node(table_node)),
                                   column_index, descending, 2);
}

BOOL ui_app_table_sort_float(int table_node, int column_index, int descending) {
    return sort_table_from_app_kind(table_for_event_id(
                                       portable_ui_table_event_for_node(table_node)),
                                   column_index, descending, 3);
}

BOOL ui_app_table_sort_bool(int table_node, int column_index, int descending) {
    return sort_table_from_app_kind(table_for_event_id(
                                       portable_ui_table_event_for_node(table_node)),
                                   column_index, descending, 4);
}

BOOL ui_app_table_filter_text(int table_node, int destination_table_id,
                              int column_index, const char *query_data,
                              unsigned __int64 query_length) {
    return filter_table_from_app(table_for_event_id(
                                     portable_ui_table_event_for_node(table_node)),
                                 destination_table_id, column_index, query_data,
                                 query_length, 0);
}

BOOL ui_app_table_filter_text_ex(int table_node, int destination_table_id,
                                 int column_index, const char *query_data,
                                 unsigned __int64 query_length, int mode) {
    return filter_table_from_app(table_for_event_id(
                                     portable_ui_table_event_for_node(table_node)),
                                 destination_table_id, column_index, query_data,
                                 query_length, mode);
}

BOOL ui_app_table_filter_int(int table_node, int destination_table_id,
                             int column_index, long long query) {
    return filter_table_from_app_int(table_for_event_id(
                                         portable_ui_table_event_for_node(table_node)),
                                     destination_table_id, column_index, query);
}

BOOL ui_app_table_filter_uint(int table_node, int destination_table_id,
                              int column_index, unsigned __int64 query) {
    return filter_table_from_app_uint(table_for_event_id(
                                          portable_ui_table_event_for_node(table_node)),
                                      destination_table_id, column_index, query);
}

BOOL ui_app_table_filter_float(int table_node, int destination_table_id,
                               int column_index, double query) {
    return filter_table_from_app_float(table_for_event_id(
                                           portable_ui_table_event_for_node(table_node)),
                                       destination_table_id, column_index, query);
}

BOOL ui_app_table_filter_bool(int table_node, int destination_table_id,
                              int column_index, unsigned char query) {
    return filter_table_from_app_bool(table_for_event_id(
                                          portable_ui_table_event_for_node(table_node)),
                                      destination_table_id, column_index, query);
}

BOOL ui_app_table_refresh(int table_node) {
    int event_id = portable_ui_table_event_for_node(table_node);
    TableSpec *table = table_for_event_id(event_id);
    if (table == 0) {
        return 0;
    }
    ui_table_refresh_app(event_id);
    return 1;
}

/* Retained UI convenience binding: use the node id returned by ui_app_* and
 * let the runtime route to the existing event-id binding implementation. */
int ui_app_bind_app_state(int node_id, const char *key_data,
                          unsigned __int64 key_length) {
    int event_id;
    int result;
    if (key_data == 0 || key_length == 0 || key_length > 64) return 0;
    /* Retained node binding attaches after the application model may already
     * have been loaded. Preserve that model and refresh the native projection;
     * the legacy event-id binders retain their initialise-from-control contract. */
    portable_ui_preserve_app_binding = 1;
    if (node_id > 0 && node_id <= 127 && portable_ui_select_bound[node_id]) {
        event_id = portable_ui_select_event_ids[node_id];
        result = ui_select_bind_app_state(event_id, key_data, key_length);
        portable_ui_preserve_app_binding = 0;
        if (result) ui_select_refresh_app_state(event_id);
        return result;
    }
    if (node_id > 0 && node_id <= 127 && portable_ui_list_bound[node_id]) {
        event_id = portable_ui_list_event_ids[node_id];
        result = ui_list_bind_app_state(event_id, key_data, key_length);
        portable_ui_preserve_app_binding = 0;
        if (result) ui_list_refresh_app_state(event_id);
        return result;
    }
    if (node_id > 0 && node_id <= 127 && portable_ui_table_bound[node_id]) {
        event_id = portable_ui_table_event_ids[node_id];
        result = ui_table_bind_app_state(event_id, key_data, key_length);
        portable_ui_preserve_app_binding = 0;
        if (result) ui_table_refresh_app_state(event_id);
        return result;
    }
    event_id = portable_ui_event_for_node(node_id);
    if (event_id == 0) {
        portable_ui_preserve_app_binding = 0;
        return 0;
    }
    if (input_for_event_id(event_id) != 0) {
        ui_input_bind_app_state(event_id, key_data, key_length);
        portable_ui_preserve_app_binding = 0;
        ui_input_refresh_app_state(event_id);
        return 1;
    }
    result = ui_checkbox_bind_app_state(event_id, key_data, key_length);
    portable_ui_preserve_app_binding = 0;
    if (result) ui_checkbox_refresh_app_state(event_id);
    return result;
}

void ui_app_refresh_app_state(int node_id) {
    int event_id;
    if (node_id > 0 && node_id <= 127 && portable_ui_select_bound[node_id]) {
        ui_select_refresh_app_state(portable_ui_select_event_ids[node_id]);
        return;
    }
    if (node_id > 0 && node_id <= 127 && portable_ui_list_bound[node_id]) {
        ui_list_refresh_app_state(portable_ui_list_event_ids[node_id]);
        return;
    }
    if (node_id > 0 && node_id <= 127 && portable_ui_table_bound[node_id]) {
        ui_table_refresh_app_state(portable_ui_table_event_ids[node_id]);
        return;
    }
    event_id = portable_ui_event_for_node(node_id);
    if (event_id == 0) return;
    if (input_for_event_id(event_id) != 0) {
        ui_input_refresh_app_state(event_id);
    } else {
        ui_checkbox_refresh_app_state(event_id);
    }
}

BOOL ui_app_table_clear(int table_node) {
    int event_id = portable_ui_table_event_for_node(table_node);
    TableSpec *table = table_for_event_id(event_id);
    if (table == 0) {
        return 0;
    }
    ui_table_clear(event_id);
    return table->row_count == 0 && table->selected_row == -1;
}

int ui_app_table_row_count(int table_node) {
    return ui_table_row_count(portable_ui_table_event_for_node(table_node));
}

int ui_app_table_selected_row(int table_node) {
    return ui_table_selected_row(portable_ui_table_event_for_node(table_node));
}

BOOL ui_app_table_set_selected_row(int table_node, int row_index) {
    int event_id = portable_ui_table_event_for_node(table_node);
    TableSpec *table = table_for_event_id(event_id);
    if (table == 0 || row_index < -1 || row_index >= table->row_count) {
        return 0;
    }
    ui_table_set_selected_row(event_id, row_index);
    return table->selected_row == row_index;
}

int ui_app_checkbox(int parent, const char *label_data,
                    unsigned __int64 label_length, int event_id,
                    int width, int height, UINT text_color,
                    UINT background_color, int corner_radius,
                    int stretch, int checked) {
    int before_buttons;
    if (!portable_ui_parent_is_current(parent)) {
        return 0;
    }
    before_buttons = desktop.button_count;
    add_layout_checkbox(label_data, label_length, event_id, width, height,
                        text_color, background_color, corner_radius,
                        stretch, checked);
    if (desktop.button_count != before_buttons + 1) {
        return 0;
    }
    return portable_ui_bind_event_node(portable_ui_allocate_node(), event_id);
}

int ui_app_button(int parent, const char *label_data, unsigned __int64 label_length,
                  int event_id, int width, int height, UINT text_color,
                  UINT background_color, int corner_radius, int stretch) {
    int before_buttons;
    int node;
    if (!portable_ui_parent_is_current(parent)) {
        return 0;
    }
    before_buttons = desktop.button_count;
    ui_layout_event_button(label_data, label_length, event_id, width, height,
                           text_color, background_color, corner_radius, stretch);
    if (desktop.button_count != before_buttons + 1) {
        return 0;
    }
    node = portable_ui_allocate_node();
    return portable_ui_bind_event_node(node, event_id);
}

BOOL ui_app_tooltip(int node, const char *text_data, unsigned __int64 text_length,
                    int width, int height, UINT text_color,
                    UINT background_color, int corner_radius) {
    int event_id = portable_ui_event_for_node(node);
    int before_count;
    if (event_id == 0 || text_data == 0) {
        return 0;
    }
    before_count = desktop.tooltip_count;
    ui_tooltip(event_id, text_data, text_length, width, height,
               text_color, background_color, corner_radius);
    return desktop.tooltip_count == before_count + 1;
}

BOOL ui_app_end(int node) {
    if (!portable_ui_active || portable_ui_depth <= 0 ||
        portable_ui_stack[portable_ui_depth - 1] != node) {
        return 0;
    }
    ui_layout_end();
    portable_ui_depth -= 1;
    if (desktop.layout_depth <= 0) {
        portable_ui_active = 0;
        portable_ui_depth = 0;
        return 1;
    }
    if (portable_ui_depth == 0) {
        portable_ui_active = 0;
        return 0;
    }
    return 1;
}

static ButtonSpec *event_button_for_id(int event_id) {
    int index;
    for (index = 0; index < desktop.button_count; index += 1) {
        ButtonSpec *button = &desktop.buttons[index];
        if (button->emits_event && button->event_id == event_id) {
            return button;
        }
    }
    return 0;
}

static BOOL event_control_exists(int event_id) {
    int index;
    if (event_id == 0) {
        return 0;
    }
    if (event_button_for_id(event_id) != 0) {
        return 1;
    }
    for (index = 0; index < desktop.input_count; index += 1) {
        if (desktop.inputs[index].event_id == event_id) {
            return 1;
        }
    }
    for (index = 0; index < desktop.select_count; index += 1) {
        if (desktop.selects[index].event_id == event_id) {
            return 1;
        }
    }
    for (index = 0; index < desktop.list_count; index += 1) {
        if (desktop.lists[index].event_id == event_id) {
            return 1;
        }
    }
    for (index = 0; index < desktop.table_count; index += 1) {
        if (desktop.tables[index].event_id == event_id) {
            return 1;
        }
    }
    return 0;
}

/* Programmatic event dispatch shares the exact callback contract used by
 * native input messages. It is useful for deterministic workflows and
 * headless tests, while refusing unknown event ids. */
int ui_dispatch_event(int event_id) {
#if JADREN_UI_HAS_EVENT_CALLBACK
    if (!event_control_exists(event_id)) {
        return 0;
    }
    /* Keep programmatic dispatch identical to the native WM_COMMAND path:
     * publish the control value before user code runs and refresh any bound
     * controls after the callback mutates the model. */
    sync_app_bindings_from_native();
    sync_state_bindings_for_event(event_id);
    jadren_ui_on_click(event_id);
    refresh_app_bindings();
    return 1;
#else
    (void)event_id;
    return 0;
#endif
}

static void redraw_button(ButtonSpec *button);

void ui_set_status(const char *text_data, unsigned __int64 text_length) {
    LabelSpec *status;
    if (desktop.status_index < 0 || desktop.status_index >= desktop.label_count) {
        return;
    }
    status = &desktop.labels[desktop.status_index];
    copy_utf8((JadrenString){text_data, (LONGLONG)text_length}, status->text, 384);
    if (status->handle != 0) {
        SetWindowTextW(status->handle, status->text);
        RedrawWindow(status->handle, 0, 0, RDW_INVALIDATE | RDW_UPDATENOW);
    }
}

void ui_set_button_text(int event_id, const char *text_data, unsigned __int64 text_length) {
    ButtonSpec *button = event_button_for_id(event_id);
    if (button == 0) {
        return;
    }
    copy_utf8((JadrenString){text_data, (LONGLONG)text_length}, button->label, 160);
    if (button->handle != 0) {
        SetWindowTextW(button->handle, button->label);
        redraw_button(button);
    }
}

void ui_set_button_enabled(int event_id, int enabled) {
    ButtonSpec *button = event_button_for_id(event_id);
    if (button == 0) {
        return;
    }
    button->disabled = enabled == 0;
    button->hovered = 0;
    button->pressed = 0;
    if (button->handle != 0) {
        EnableWindow(button->handle, enabled != 0);
        redraw_button(button);
    }
}

static int state_binding_value(const StateBinding *binding) {
    ButtonSpec *button;
    SelectSpec *select;
    ListSpec *list;
    TableSpec *table;
    InputSpec *input;
    if (binding == 0) {
        return 0;
    }
    switch (binding->mode) {
        case UI_STATE_BIND_CHECKED:
            button = event_toggle_for_id(binding->event_id);
            return button != 0 && button->active ? 1 : 0;
        case UI_STATE_BIND_INDEX:
            select = select_for_event_id(binding->event_id);
            if (select != 0) return select->selected_index;
            list = list_for_event_id(binding->event_id);
            if (list != 0) return list->selected_index;
            table = table_for_event_id(binding->event_id);
            return table != 0 ? table->selected_row : -1;
        case UI_STATE_BIND_INPUT_LENGTH:
            input = input_for_event_id(binding->event_id);
            if (input == 0) return 0;
            sync_input_text(input);
            return (int)input_utf8_length(input);
        case UI_STATE_BIND_COUNT:
            list = list_for_event_id(binding->event_id);
            if (list != 0) return list->item_count;
            table = table_for_event_id(binding->event_id);
            return table != 0 ? table->row_count : 0;
        default:
            return 0;
    }
}

static void apply_state_binding(const StateBinding *binding) {
    ButtonSpec *button;
    SelectSpec *select;
    ListSpec *list;
    TableSpec *table;
    int value;
    if (binding == 0 || binding->slot < 0 || binding->slot >= 32) {
        return;
    }
    value = desktop.state_slots[binding->slot];
    switch (binding->mode) {
        case UI_STATE_BIND_CHECKED:
            button = event_toggle_for_id(binding->event_id);
            if (button != 0) {
                button->active = value != 0;
                redraw_button(button);
            }
            break;
        case UI_STATE_BIND_INDEX:
            select = select_for_event_id(binding->event_id);
            if (select != 0 && value >= -1 && value < select->option_count) {
                select->selected_index = value;
                if (select->handle != 0) {
                    SendMessageW(select->handle, CB_SETCURSEL,
                                 (WPARAM)(unsigned __int64)value, 0);
                }
                break;
            }
            list = list_for_event_id(binding->event_id);
            if (list != 0 && value >= -1 && value < list->item_count) {
                list->selected_index = value;
                if (list->handle != 0) {
                    SendMessageW(list->handle, LB_SETCURSEL,
                                 (WPARAM)(unsigned __int64)value, 0);
                }
                break;
            }
            table = table_for_event_id(binding->event_id);
            if (table != 0 && value >= -1 && value < table->row_count) {
                ui_table_set_selected_row(binding->event_id, value);
            }
            break;
        case UI_STATE_BIND_INPUT_LENGTH:
        case UI_STATE_BIND_COUNT:
        default:
            break;
    }
}

static void sync_state_bindings_for_event(int event_id) {
    int index;
    for (index = 0; index < desktop.state_binding_count; index += 1) {
        StateBinding *binding = &desktop.state_bindings[index];
        if (binding->event_id == event_id) {
            if (binding->mode == UI_STATE_BIND_TEXT) {
                InputSpec *input = input_for_event_id(event_id);
                if (input != 0) {
                    sync_input_text(input);
                    copy_wide_text(input->text, desktop.state_text[binding->slot], 256);
                }
            } else {
                desktop.state_slots[binding->slot] = state_binding_value(binding);
            }
        }
    }
}

void ui_state_bind(int event_id, int slot, int mode) {
    int index;
    StateBinding *binding = 0;
    if (slot < 0 || slot >= 32 || mode < UI_STATE_BIND_CHECKED ||
        mode > UI_STATE_BIND_COUNT) {
        return;
    }
    for (index = 0; index < desktop.state_binding_count; index += 1) {
        StateBinding *candidate = &desktop.state_bindings[index];
        if (candidate->event_id == event_id && candidate->mode == mode) {
            binding = candidate;
            break;
        }
    }
    if (binding == 0) {
        if (desktop.state_binding_count >= 64) {
            return;
        }
        binding = &desktop.state_bindings[desktop.state_binding_count];
        desktop.state_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->slot = slot;
    binding->mode = mode;
    if (mode == UI_STATE_BIND_INPUT_LENGTH || mode == UI_STATE_BIND_COUNT) {
        desktop.state_slots[slot] = state_binding_value(binding);
    } else {
        apply_state_binding(binding);
    }
}

void ui_state_bind_text(int event_id, int slot) {
    int index;
    StateBinding *binding = 0;
    InputSpec *input;
    if (slot < 0 || slot >= 32) return;
    for (index = 0; index < desktop.state_binding_count; index += 1) {
        StateBinding *candidate = &desktop.state_bindings[index];
        if (candidate->event_id == event_id && candidate->mode == UI_STATE_BIND_TEXT) {
            binding = candidate;
            break;
        }
    }
    if (binding == 0) {
        if (desktop.state_binding_count >= 64) return;
        binding = &desktop.state_bindings[desktop.state_binding_count];
        desktop.state_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->slot = slot;
    binding->mode = UI_STATE_BIND_TEXT;
    input = input_for_event_id(event_id);
    if (input != 0) {
        sync_input_text(input);
        copy_wide_text(input->text, desktop.state_text[slot], 256);
    } else {
        desktop.state_text[slot][0] = L'\0';
    }
}

unsigned __int64 ui_state_text_length(int slot) {
    if (slot < 0 || slot >= 32) return 0;
    return wide_utf8_length(desktop.state_text[slot]);
}

unsigned __int64 ui_state_text_read(int slot, unsigned char *output_data,
                                    unsigned __int64 output_length) {
    if (slot < 0 || slot >= 32) return 0;
    return copy_wide_utf8(desktop.state_text[slot], output_data, output_length);
}

int ui_state_text_set(int slot, const char *text_data, unsigned __int64 text_length) {
    int index;
    if (slot < 0 || slot >= 32 || (text_data == 0 && text_length > 0)) return 0;
    copy_utf8((JadrenString){text_data, (LONGLONG)text_length}, desktop.state_text[slot], 256);
    for (index = 0; index < desktop.state_binding_count; index += 1) {
        StateBinding *binding = &desktop.state_bindings[index];
        if (binding->slot == slot && binding->mode == UI_STATE_BIND_TEXT) {
            InputSpec *input = input_for_event_id(binding->event_id);
            if (input != 0) {
                copy_wide_text(desktop.state_text[slot], input->text, 384);
                if (input->handle != 0) {
                    input->updating = 1;
                    SetWindowTextW(input->handle, input->text);
                    input->updating = 0;
                }
                sync_input_app_state(input);
            }
        }
    }
    return 1;
}

int ui_state_get(int slot) {
    if (slot < 0 || slot >= 32) {
        return 0;
    }
    return desktop.state_slots[slot];
}

void ui_state_set(int slot, int value) {
    int index;
    if (slot < 0 || slot >= 32) {
        return;
    }
    desktop.state_slots[slot] = value;
    for (index = 0; index < desktop.state_binding_count; index += 1) {
        if (desktop.state_bindings[index].slot == slot) {
            apply_state_binding(&desktop.state_bindings[index]);
        }
    }
}

static LabelSpec *label_for_handle(HWND handle) {
    int index;
    for (index = 0; index < desktop.label_count; index += 1) {
        if (desktop.labels[index].handle == handle) {
            return &desktop.labels[index];
        }
    }
    return 0;
}

static ButtonSpec *button_for_id(UINT id) {
    int index;
    for (index = 0; index < desktop.button_count; index += 1) {
        if (desktop.buttons[index].id == id) {
            return &desktop.buttons[index];
        }
    }
    return 0;
}

static ButtonSpec *button_for_handle(HWND handle) {
    int index;
    for (index = 0; index < desktop.button_count; index += 1) {
        if (desktop.buttons[index].handle == handle) {
            return &desktop.buttons[index];
        }
    }
    return 0;
}

static TooltipSpec *tooltip_for_event_id(int event_id) {
    int index;
    for (index = 0; index < desktop.tooltip_count; index += 1) {
        if (desktop.tooltips[index].event_id == event_id) {
            return &desktop.tooltips[index];
        }
    }
    return 0;
}

/* Buttons already have a Jadren subclass for hover, press and events. Relay
 * its input to the standard tooltip controller instead of stacking a second
 * legacy window-procedure subclass on the same BUTTON. */
static void relay_tooltip_event(ButtonSpec *button, HWND window, UINT message,
                                WPARAM wparam, LPARAM lparam) {
    TooltipSpec *tooltip;
    MSG event = {window, message, wparam, lparam, 0, {0, 0}, 0};
    if (button == 0 || !button->emits_event) {
        return;
    }
    tooltip = tooltip_for_event_id(button->event_id);
    if (tooltip != 0 && tooltip->handle != 0) {
        GetCursorPos(&event.pt);
        SendMessageW(tooltip->handle, TTM_RELAYEVENT, 0,
                     (LPARAM)(unsigned __int64)&event);
    }
}

static void redraw_button(ButtonSpec *button) {
    if (button != 0 && button->handle != 0) {
        RedrawWindow(button->handle, 0, 0, RDW_INVALIDATE | RDW_UPDATENOW);
    }
}

/* portable_ui_event_contract: the native button subclass is the Win32
 * adapter for jadren-ui::InteractionModel. WM_MOUSEMOVE/WM_MOUSELEAVE map to
 * hover enter/leave, WM_LBUTTONDOWN/WM_LBUTTONUP map to press/release, and
 * WM_CANCELMODE/WM_CAPTURECHANGED cancel a held press. Only a confirmed
 * BN_CLICKED is forwarded to the Jadren callback, so hover never mutates
 * application state by accident. */
/* Windows does not reliably set ODS_HOTLIGHT for every owner-drawn BUTTON.
 * Track child mouse enter/leave ourselves and only use ODS_HOTLIGHT as an
 * additional signal. This makes Jadren hover deterministic across themes. */
static LRESULT desktop_button_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    ButtonSpec *button = button_for_handle(window);
    if (button != 0 && button->is_checkbox && message == 0x00F0) {
        return (LRESULT)(button->active ? 1 : 0);
    }
    if (button != 0 && button->is_checkbox && message == 0x00F1) {
        button->active = wparam != 0;
        redraw_button(button);
        return 0;
    }
    relay_tooltip_event(button, window, message, wparam, lparam);
    if (button != 0 && !button->disabled) {
        if (message == WM_MOUSEMOVE) {
            if (!button->hovered) {
                button->hovered = 1;
                redraw_button(button);
            }
            if (!button->tracks_mouse) {
                TRACKMOUSEEVENT event = {(DWORD)sizeof(TRACKMOUSEEVENT), TME_LEAVE, window, 0};
                button->tracks_mouse = TrackMouseEvent(&event) != 0;
            }
        } else if (message == WM_MOUSELEAVE) {
            button->tracks_mouse = 0;
            if (button->hovered || button->pressed) {
                button->hovered = 0;
                button->pressed = 0;
                redraw_button(button);
            }
        } else if (message == WM_LBUTTONDOWN) {
            if (!button->pressed) {
                button->pressed = 1;
                redraw_button(button);
            }
        } else if (message == WM_LBUTTONUP || message == WM_CANCELMODE ||
                   message == WM_CAPTURECHANGED) {
            if (button->pressed) {
                button->pressed = 0;
                redraw_button(button);
            }
        }
    }
    if (button != 0 && button->default_proc != 0) {
        return CallWindowProcW(button->default_proc, window, message, wparam, lparam);
    }
    return DefWindowProcW(window, message, wparam, lparam);
}

/* EDIT sends WM_COMMAND notifications inconsistently across input methods.
 * Subclass the native field and call Jadren only after its default processor
 * has committed the edit, so keyboard text, paste, cut and clear reliably
 * reach the Jadren event handler. */
static LRESULT desktop_input_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    InputSpec *input = input_for_handle(window);
    if (input != 0 && message == WM_JADREN_SET_INPUT_TEXT) {
        input->updating = 1;
        SetWindowTextW(window, input->text);
        input->updating = 0;
        return 0;
    }
    LRESULT result = input != 0 && input->default_proc != 0
        ? CallWindowProcW(input->default_proc, window, message, wparam, lparam)
        : DefWindowProcW(window, message, wparam, lparam);
#if JADREN_UI_HAS_EVENT_CALLBACK
    if (input != 0 && desktop.events_ready && !input->updating &&
        message == WM_SETTEXT) {
#if JADREN_UI_HAS_FILE_RUNTIME
        unsigned char output[256];
        unsigned __int64 copied;
#endif
        /* The default EDIT proc owns WM_SETTEXT marshalling. We never inspect
         * its sender-owned lParam; WM_GETTEXT reads the committed local value
         * after the default processor has returned. */
        sync_input_text(input);
#if JADREN_UI_HAS_FILE_RUNTIME
        {
            InputAppBinding *binding = input_app_binding_for_event_id(input->event_id);
            if (binding != 0 && binding->kind == JADREN_APP_BIND_TEXT) {
                copied = copy_wide_utf8(input->text, output, sizeof(output));
                (void)app_state_set_text(binding->key, binding->key_length,
                                         (const char *)output, copied);
            }
        }
#endif
        sync_state_bindings_for_event(input->event_id);
        jadren_ui_on_click(input->event_id);
        refresh_app_bindings();
        return result;
    }
    if (input != 0 && desktop.events_ready && !input->updating &&
        (message == WM_CHAR || message == WM_PASTE || message == WM_CUT ||
         message == WM_CLEAR)) {
        sync_input_text(input);
        sync_input_app_state(input);
        sync_state_bindings_for_event(input->event_id);
        jadren_ui_on_click(input->event_id);
        refresh_app_bindings();
    }
#endif
    return result;
}

static void dispose_brushes(void) {
    int index;
    if (desktop.background_brush != 0) {
        DeleteObject(desktop.background_brush);
        desktop.background_brush = 0;
    }
    if (desktop.border_brush != 0) {
        DeleteObject(desktop.border_brush);
        desktop.border_brush = 0;
    }
    for (index = 0; index < desktop.label_count; index += 1) {
        if (desktop.labels[index].brush != 0) {
            DeleteObject(desktop.labels[index].brush);
            desktop.labels[index].brush = 0;
        }
    }
    for (index = 0; index < desktop.button_count; index += 1) {
        if (desktop.buttons[index].brush != 0) {
            DeleteObject(desktop.buttons[index].brush);
            desktop.buttons[index].brush = 0;
        }
        if (desktop.buttons[index].font != 0) {
            DeleteObject(desktop.buttons[index].font);
            desktop.buttons[index].font = 0;
        }
    }
    for (index = 0; index < desktop.input_count; index += 1) {
        if (desktop.inputs[index].brush != 0) {
            DeleteObject(desktop.inputs[index].brush);
            desktop.inputs[index].brush = 0;
        }
    }
    for (index = 0; index < desktop.select_count; index += 1) {
        if (desktop.selects[index].brush != 0) {
            DeleteObject(desktop.selects[index].brush);
            desktop.selects[index].brush = 0;
        }
    }
    for (index = 0; index < desktop.scroll_count; index += 1) {
        if (desktop.scrolls[index].brush != 0) {
            DeleteObject(desktop.scrolls[index].brush);
            desktop.scrolls[index].brush = 0;
        }
    }
    for (index = 0; index < desktop.list_count; index += 1) {
        if (desktop.lists[index].brush != 0) {
            DeleteObject(desktop.lists[index].brush);
            desktop.lists[index].brush = 0;
        }
    }
    for (index = 0; index < desktop.table_count; index += 1) {
        if (desktop.tables[index].brush != 0) {
            DeleteObject(desktop.tables[index].brush);
            desktop.tables[index].brush = 0;
        }
    }
}

static void dispose_images(void) {
    int index;
    for (index = 0; index < desktop.image_count; index += 1) {
        if (desktop.images[index].image != 0) {
            GdipDisposeImage(desktop.images[index].image);
            desktop.images[index].image = 0;
        }
    }
    if (gdiplus_token != 0) {
        GdiplusShutdown(gdiplus_token);
        gdiplus_token = 0;
    }
}

static int load_images(void) {
    int index;
    GdiplusStartupInput input = {1, 0, 0, 0};
    if (desktop.image_count == 0) {
        return 1;
    }
    if (GdiplusStartup(&gdiplus_token, &input, 0) != 0 || gdiplus_token == 0) {
        return 0;
    }
    for (index = 0; index < desktop.image_count; index += 1) {
        ImageSpec *image = &desktop.images[index];
        wchar_t resolved_path[640];
        resolve_image_path(image->path, resolved_path, 640);
        if (resolved_path[0] == L'\0' ||
            GdipCreateBitmapFromFile(resolved_path, &image->image) != 0 || image->image == 0) {
            dispose_images();
            return 0;
        }
    }
    return 1;
}

static void draw_images(HDC device_context) {
    int index;
    GpGraphics graphics = 0;
    if (desktop.image_count == 0 || device_context == 0 ||
        GdipCreateFromHDC(device_context, &graphics) != 0 || graphics == 0) {
        return;
    }
    for (index = 0; index < desktop.image_count; index += 1) {
        ImageSpec *image = &desktop.images[index];
        if (image->image != 0) {
            GdipDrawImageRectI(graphics, image->image,
                               image->x, image->y, image->width, image->height);
        }
    }
    GdipDeleteGraphics(graphics);
}

static void apply_corner_radius(HWND handle, int width, int height, int corner_radius) {
    HRGN region;
    int maximum_radius;
    int radius;
    if (handle == 0 || width <= 0 || height <= 0 || corner_radius <= 0) {
        return;
    }
    maximum_radius = width < height ? width / 2 : height / 2;
    radius = clamp_dimension(corner_radius, 0, maximum_radius);
    if (radius <= 0) {
        return;
    }
    region = CreateRoundRectRgn(0, 0, width, height, radius * 2, radius * 2);
    if (region != 0 && SetWindowRgn(handle, region, 0) == 0) {
        DeleteObject(region);
    }
}

static int distributed_button_count(void) {
    int count = 0;
    int index;
    for (index = 0; index < desktop.button_count; index += 1) {
        if (desktop.buttons[index].distributes_horizontally) {
            count += 1;
        }
    }
    return count;
}

static int horizontal_button_offset(int index, int width_delta) {
    int count = distributed_button_count();
    int distributed_index = 0;
    int cursor;
    if (count <= 1 || !desktop.buttons[index].distributes_horizontally) {
        return 0;
    }
    for (cursor = 0; cursor < index; cursor += 1) {
        if (desktop.buttons[cursor].distributes_horizontally) {
            distributed_index += 1;
        }
    }
    return (width_delta * distributed_index) / (count - 1);
}

static int aligned_cross_position(int origin, int available, int requested, int align) {
    int size = requested < available ? requested : available;
    if (align == UI_ALIGN_CENTER) {
        return origin + (available - size) / 2;
    }
    if (align == UI_ALIGN_END) {
        return origin + available - size;
    }
    return origin;
}

static void move_layout_label(int index, int x, int y, int width, int height) {
    LabelSpec *label;
    if (index < 0 || index >= desktop.label_count) {
        return;
    }
    label = &desktop.labels[index];
    MoveWindow(label->handle, x, y, width, height, 0);
    apply_corner_radius(label->handle, width, height, label->corner_radius);
}

static void move_layout_button(int index, int x, int y, int width, int height) {
    if (index < 0 || index >= desktop.button_count) {
        return;
    }
    desktop.buttons[index].current_x = x;
    desktop.buttons[index].current_y = y;
    MoveWindow(desktop.buttons[index].handle, x, y, width, height, 0);
}

static void move_layout_input(int index, int x, int y, int width, int height) {
    InputSpec *input;
    if (index < 0 || index >= desktop.input_count) {
        return;
    }
    input = &desktop.inputs[index];
    input->x = x;
    input->y = y;
    input->width = width;
    input->height = height;
    MoveWindow(input->handle, x, y, width, height, 0);
    apply_corner_radius(input->handle, width, height, input->corner_radius);
}

static void move_layout_select(int index, int x, int y, int width, int height) {
    SelectSpec *select;
    if (index < 0 || index >= desktop.select_count) {
        return;
    }
    select = &desktop.selects[index];
    select->x = x;
    select->y = y;
    select->width = width;
    select->height = height;
    MoveWindow(select->handle, x, y, width, select_window_height(select), 0);
    apply_corner_radius(select->handle, width, height, select->corner_radius);
}

static void move_layout_list(int index, int x, int y, int width, int height) {
    ListSpec *list;
    if (index < 0 || index >= desktop.list_count) {
        return;
    }
    list = &desktop.lists[index];
    list->x = x;
    list->y = y;
    list->width = width;
    list->height = height;
    MoveWindow(list->handle, x, y, width, height, 0);
    apply_corner_radius(list->handle, width, height, list->corner_radius);
}

static void move_layout_table(int index, int x, int y, int width, int height) {
    TableSpec *table;
    if (index < 0 || index >= desktop.table_count) {
        return;
    }
    table = &desktop.tables[index];
    table->x = x;
    table->y = y;
    table->width = width;
    table->height = height;
    MoveWindow(table->handle, x, y, width, height, 0);
    apply_corner_radius(table->handle, width, height, table->corner_radius);
}

static void layout_node(int node_index, int x, int y, int width, int height) {
    LayoutNode *node;
    int content_x;
    int content_y;
    int content_width;
    int content_height;
    int cursor;
    int index;
    int primary_total = 0;
    if (node_index < 0 || node_index >= desktop.layout_count || width <= 0 || height <= 0) {
        return;
    }
    node = &desktop.layouts[node_index];
    node->x = x;
    node->y = y;
    node->width = width;
    node->height = height;
    if (node->panel_label >= 0) {
        move_layout_label(node->panel_label, x, y, width, height);
    }
    content_x = x + node->padding;
    content_y = y + node->padding;
    content_width = clamp_dimension(width - node->padding * 2, 1, 4000);
    content_height = clamp_dimension(height - node->padding * 2, 1, 4000);
    for (index = 0; index < node->child_count; index += 1) {
        LayoutChild *child = &node->children[index];
        primary_total += node->orientation == LAYOUT_COLUMN ? child->height : child->width;
        if (index + 1 < node->child_count) {
            primary_total += node->gap;
        }
    }
    cursor = node->orientation == LAYOUT_COLUMN
        ? content_y
        : aligned_cross_position(content_x, content_width, primary_total, node->align);
    for (index = 0; index < node->child_count; index += 1) {
        LayoutChild *child = &node->children[index];
        int child_width;
        int child_height;
        int child_x;
        int child_y;
        if (node->orientation == LAYOUT_COLUMN) {
            child_width = child->stretch
                ? content_width
                : clamp_dimension(child->width, 1, content_width);
            child_height = clamp_dimension(child->height, 1, content_height);
            child_x = aligned_cross_position(content_x, content_width, child_width, node->align);
            child_y = cursor;
            cursor += child_height + (index + 1 < node->child_count ? node->gap : 0);
        } else {
            child_width = clamp_dimension(child->width, 1, content_width);
            child_height = child->stretch
                ? content_height
                : clamp_dimension(child->height, 1, content_height);
            child_x = cursor;
            child_y = content_y + (content_height - child_height) / 2;
            cursor += child_width + (index + 1 < node->child_count ? node->gap : 0);
        }
        if (child->kind == LAYOUT_CHILD_LABEL) {
            move_layout_label(child->index, child_x, child_y, child_width, child_height);
        } else if (child->kind == LAYOUT_CHILD_BUTTON) {
            move_layout_button(child->index, child_x, child_y, child_width, child_height);
        } else if (child->kind == LAYOUT_CHILD_INPUT) {
            move_layout_input(child->index, child_x, child_y, child_width, child_height);
        } else if (child->kind == LAYOUT_CHILD_SELECT) {
            move_layout_select(child->index, child_x, child_y, child_width, child_height);
        } else if (child->kind == LAYOUT_CHILD_LIST) {
            move_layout_list(child->index, child_x, child_y, child_width, child_height);
        } else if (child->kind == LAYOUT_CHILD_TABLE) {
            move_layout_table(child->index, child_x, child_y, child_width, child_height);
        } else if (child->kind == LAYOUT_CHILD_NODE) {
            layout_node(child->index, child_x, child_y, child_width, child_height);
        }
    }
}

static void layout_controls(HWND window) {
    RECT client;
    int width_delta;
    int index;
    if (GetClientRect(window, &client) == 0) {
        return;
    }
    if (client.right <= client.left || client.bottom <= client.top) {
        return;
    }
    if (desktop.base_client_width == 0 || desktop.base_client_height == 0) {
        desktop.base_client_width = client.right - client.left;
        desktop.base_client_height = client.bottom - client.top;
    }
    width_delta = (client.right - client.left) - desktop.base_client_width;
    for (index = 0; index < desktop.label_count; index += 1) {
        LabelSpec *label = &desktop.labels[index];
        if (label->layout_managed) {
            continue;
        }
        int width = label->fills_width
            ? clamp_dimension(label->width + width_delta, 1, 4000)
            : label->width;
        /* Do not repaint each child while the user is dragging the window.
         * A single parent redraw below clears their old rectangles before
         * the final positions are painted, preventing stale button pixels. */
        MoveWindow(label->handle, label->x, label->y, width, label->height, 0);
        apply_corner_radius(label->handle, width, label->height, label->corner_radius);
    }
    for (index = 0; index < desktop.button_count; index += 1) {
        ButtonSpec *button = &desktop.buttons[index];
        if (button->layout_managed) {
            continue;
        }
        int x = button->x;
        if (button->right_anchored) {
            x = (client.right - client.left) - button->x - button->width;
        } else if (button->distributes_horizontally) {
            x += horizontal_button_offset(index, width_delta);
        }
        button->current_x = x;
        button->current_y = button->y;
        MoveWindow(button->handle, x, button->y, button->width, button->height, 0);
    }
    for (index = 0; index < desktop.input_count; index += 1) {
        InputSpec *input = &desktop.inputs[index];
        if (input->layout_managed) {
            continue;
        }
        MoveWindow(input->handle, input->x, input->y, input->width, input->height, 0);
        apply_corner_radius(input->handle, input->width, input->height, input->corner_radius);
    }
    for (index = 0; index < desktop.select_count; index += 1) {
        SelectSpec *select = &desktop.selects[index];
        if (select->layout_managed) {
            continue;
        }
        MoveWindow(select->handle, select->x, select->y, select->width,
                   select_window_height(select), 0);
    }
    for (index = 0; index < desktop.scroll_count; index += 1) {
        ScrollSpec *scroll = &desktop.scrolls[index];
        MoveWindow(scroll->handle, scroll->x, scroll->y, scroll->width, scroll->height, 0);
        apply_corner_radius(scroll->handle, scroll->width, scroll->height, scroll->corner_radius);
    }
    for (index = 0; index < desktop.list_count; index += 1) {
        ListSpec *list = &desktop.lists[index];
        if (list->layout_managed) {
            continue;
        }
        MoveWindow(list->handle, list->x, list->y, list->width, list->height, 0);
    }
    for (index = 0; index < desktop.table_count; index += 1) {
        TableSpec *table = &desktop.tables[index];
        if (table->layout_managed) {
            continue;
        }
        MoveWindow(table->handle, table->x, table->y, table->width, table->height, 0);
    }
    for (index = 0; index < desktop.layout_count; index += 1) {
        LayoutNode *node = &desktop.layouts[index];
        if (node->parent == -1) {
            int width = node->stretch
                ? clamp_dimension(node->base_width + width_delta, 1, 4000)
                : node->base_width;
            layout_node(index, node->x, node->y, width, node->base_height);
        }
    }
    RedrawWindow(window, 0, 0,
                 RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW);
}

static LRESULT desktop_window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    if (message == WM_ERASEBKGND) {
        RECT client;
        if (desktop.background_brush != 0 && GetClientRect(window, &client) != 0) {
            FillRect((HDC)wparam, &client, desktop.background_brush);
        }
        return 1;
    }
    if (message == WM_PAINT) {
        PAINTSTRUCT paint = {0};
        HDC device_context = BeginPaint(window, &paint);
        if (device_context != 0) {
            draw_images(device_context);
            EndPaint(window, &paint);
        }
        return 0;
    }
    if (message == WM_GETMINMAXINFO) {
        MINMAXINFO *min_max_info = (MINMAXINFO *)lparam;
        if (min_max_info != 0) {
            min_max_info->ptMinTrackSize.x = desktop.min_width;
            min_max_info->ptMinTrackSize.y = desktop.min_height;
            min_max_info->ptMaxTrackSize.x = desktop.max_width;
            min_max_info->ptMaxTrackSize.y = desktop.max_height;
        }
        return 0;
    }
    if (message == WM_SIZE) {
        /* A minimized window reports a zero-sized client area. Never lay out
         * child controls from that transient value, otherwise their regions
         * survive restored with zero or clipped dimensions. */
        if ((UINT)wparam != SIZE_MINIMIZED) {
            layout_controls(window);
#if JADREN_UI_HAS_EVENT_CALLBACK
            if (desktop.events_ready && desktop.resize_event_id != 0) {
                sync_app_bindings_from_native();
                jadren_ui_on_click(desktop.resize_event_id);
                refresh_app_bindings();
            }
#endif
        }
        return 0;
    }
    if (message == WM_NOTIFY) {
        NMLISTVIEW *notice = (NMLISTVIEW *)lparam;
        TableSpec *table = notice == 0 ? 0 : table_for_control_id((UINT)notice->hdr.idFrom);
        if (table != 0 && (UINT)notice->hdr.code == LVN_ITEMCHANGED &&
            !table->updating && (notice->uChanged & LVIF_STATE) != 0 &&
            (notice->uNewState & LVIS_SELECTED) != 0) {
            table->selected_row = notice->iItem;
            sync_table_app_state(table);
#if JADREN_UI_HAS_EVENT_CALLBACK
            if (desktop.events_ready) {
                sync_state_bindings_for_event(table->event_id);
                jadren_ui_on_click(table->event_id);
                refresh_app_bindings();
            }
#endif
            return 0;
        }
    }
    if (message == WM_COMMAND) {
        UINT notification = (UINT)((wparam >> 16) & 0xFFFFU);
        UINT control_id = (UINT)(wparam & 0xFFFFU);
        SelectSpec *select = select_for_control_id(control_id);
        ListSpec *list = list_for_control_id(control_id);
        if (select != 0 && notification == CBN_SELCHANGE) {
            select->selected_index = (int)SendMessageW(select->handle, CB_GETCURSEL, 0, 0);
            sync_select_app_state(select);
#if JADREN_UI_HAS_EVENT_CALLBACK
            if (desktop.events_ready) {
                sync_state_bindings_for_event(select->event_id);
                jadren_ui_on_click(select->event_id);
                refresh_app_bindings();
            }
#endif
            return 0;
        }
        if (list != 0 && notification == LBN_SELCHANGE) {
            list->selected_index = (int)SendMessageW(list->handle, LB_GETCURSEL, 0, 0);
            sync_list_app_state(list);
#if JADREN_UI_HAS_EVENT_CALLBACK
            if (desktop.events_ready) {
                sync_state_bindings_for_event(list->event_id);
                jadren_ui_on_click(list->event_id);
                refresh_app_bindings();
            }
#endif
            return 0;
        }
        if (notification != BN_CLICKED) {
            return DefWindowProcW(window, message, wparam, lparam);
        }
        ButtonSpec *button = button_for_id(control_id);
        if (button != 0 && !button->disabled) {
            int index;
            MenuSpec *popup_menu = menu_for_button_id(button->id);
            if (button->is_menu_item) {
                for (index = 0; index < desktop.button_count; index += 1) {
                    ButtonSpec *candidate = &desktop.buttons[index];
                    if (candidate->is_menu_item) {
                        candidate->active = candidate == button;
                        RedrawWindow(candidate->handle, 0, 0, RDW_INVALIDATE | RDW_UPDATENOW);
                    }
                }
            } else if (button->toggles) {
                button->active = !button->active;
                SendMessageW(button->handle, 0x00F1,
                             (WPARAM)(button->active ? 1 : 0), 0);
                sync_checkbox_app_state(button);
                RedrawWindow(button->handle, 0, 0, RDW_INVALIDATE | RDW_UPDATENOW);
            }
            if (popup_menu != 0) {
                show_popup_menu(window, popup_menu);
                return 0;
            }
#if JADREN_UI_HAS_EVENT_CALLBACK
            if (button->emits_event) {
                sync_app_bindings_from_native();
                sync_state_bindings_for_event(button->event_id);
                jadren_ui_on_click(button->event_id);
                refresh_app_bindings();
                return 0;
            }
#endif
            if (button->closes_window) {
                DestroyWindow(window);
            } else if (desktop.status_index >= 0) {
                SetWindowTextW(desktop.labels[desktop.status_index].handle, button->action);
            }
            return 0;
        }
    }
    if (message == WM_CTLCOLORSTATIC) {
        LabelSpec *label = label_for_handle((HWND)lparam);
        if (label != 0) {
            SetTextColor((HDC)wparam, win32_color(label->text_color));
            SetBkColor((HDC)wparam, win32_color(label->background_color));
            return (LRESULT)label->brush;
        }
        for (int scroll_index = 0; scroll_index < desktop.scroll_count; scroll_index += 1) {
            ScrollSpec *candidate = &desktop.scrolls[scroll_index];
            if (candidate->handle == (HWND)lparam) {
                SetTextColor((HDC)wparam, win32_color(candidate->text_color));
                SetBkColor((HDC)wparam, win32_color(candidate->background_color));
                return (LRESULT)candidate->brush;
            }
        }
    }
    if (message == WM_CTLCOLOREDIT) {
        /* EDIT does not expose its identifier through the colour message. Find
         * it by HWND instead, keeping all input colours owned by Jadren. */
        for (int input_index = 0; input_index < desktop.input_count; input_index += 1) {
            InputSpec *candidate = &desktop.inputs[input_index];
            if (candidate->handle == (HWND)lparam) {
                SetTextColor((HDC)wparam, win32_color(candidate->text_color));
                SetBkColor((HDC)wparam, win32_color(candidate->background_color));
                return (LRESULT)candidate->brush;
            }
        }
        for (int scroll_index = 0; scroll_index < desktop.scroll_count; scroll_index += 1) {
            ScrollSpec *candidate = &desktop.scrolls[scroll_index];
            if (candidate->handle == (HWND)lparam) {
                SetTextColor((HDC)wparam, win32_color(candidate->text_color));
                SetBkColor((HDC)wparam, win32_color(candidate->background_color));
                return (LRESULT)candidate->brush;
            }
        }
    }
    if (message == WM_CTLCOLORLISTBOX) {
        for (int list_index = 0; list_index < desktop.list_count; list_index += 1) {
            ListSpec *candidate = &desktop.lists[list_index];
            if (candidate->handle == (HWND)lparam) {
                SetTextColor((HDC)wparam, win32_color(candidate->text_color));
                SetBkColor((HDC)wparam, win32_color(candidate->background_color));
                return (LRESULT)candidate->brush;
            }
        }
    }
    if (message == WM_DRAWITEM) {
        DRAWITEMSTRUCT *draw_item = (DRAWITEMSTRUCT *)lparam;
        ButtonSpec *button = draw_item == 0 ? 0 : button_for_id(draw_item->CtlID);
        if (button != 0) {
            if (button->is_checkbox || button->is_switch) {
                int control_height = (draw_item->rcItem.bottom - draw_item->rcItem.top) < 22
                    ? draw_item->rcItem.bottom - draw_item->rcItem.top : 22;
                int control_width = button->is_switch ? 48 : 22;
                int control_left = button->is_switch
                    ? draw_item->rcItem.right - control_width - 2 : draw_item->rcItem.left + 2;
                int control_top = draw_item->rcItem.top +
                    ((draw_item->rcItem.bottom - draw_item->rcItem.top) - control_height) / 2;
                int control_radius = button->is_switch ? control_height / 2 : 4;
                BOOL disabled = button->disabled || (draw_item->itemState & ODS_DISABLED) != 0;
                BOOL pressed = !disabled && (button->pressed ||
                                             (draw_item->itemState & ODS_SELECTED) != 0);
                UINT fill_color = button->active
                    ? button->background_color : adjust_rgb(button->background_color, -70);
                UINT text_color = disabled ? adjust_rgb(button->text_color, -100) : button->text_color;
                RECT control_rect = {control_left, control_top,
                                     control_left + control_width, control_top + control_height};
                RECT text_rectangle = draw_item->rcItem;
                HRGN control_region = CreateRoundRectRgn(
                    control_rect.left, control_rect.top, control_rect.right, control_rect.bottom,
                    control_radius * 2, control_radius * 2);
                HBRUSH fill_brush;
                HBRUSH frame_brush;
                HBRUSH white_brush = 0;

                if (disabled) {
                    fill_color = adjust_rgb(fill_color, -35);
                } else if (pressed) {
                    fill_color = adjust_rgb(fill_color, -35);
                } else if (button->hovered || (draw_item->itemState & ODS_HOTLIGHT) != 0) {
                    fill_color = adjust_rgb(fill_color, 28);
                }
                fill_brush = CreateSolidBrush(win32_color(fill_color));
                frame_brush = CreateSolidBrush(win32_color(
                    button->active ? adjust_rgb(button->background_color, 72) : 0x94A3B8U));
                FillRect(draw_item->hDC, &draw_item->rcItem, desktop.background_brush);
                if (control_region != 0 && fill_brush != 0) {
                    FillRgn(draw_item->hDC, control_region, fill_brush);
                    if (frame_brush != 0) {
                        FrameRgn(draw_item->hDC, control_region, frame_brush, 1, 1);
                    }
                }

                if (button->is_checkbox) {
                    text_rectangle.left = control_rect.right + 10;
                } else {
                    int thumb_size = control_height - 6;
                    int thumb_left = button->active
                        ? control_rect.right - thumb_size - 3 : control_rect.left + 3;
                    HRGN thumb_region = CreateRoundRectRgn(
                        thumb_left, control_rect.top + 3,
                        thumb_left + thumb_size, control_rect.bottom - 3,
                        thumb_size, thumb_size);
                    white_brush = CreateSolidBrush(win32_color(0xFFFFFFU));
                    if (thumb_region != 0 && white_brush != 0) {
                        FillRgn(draw_item->hDC, thumb_region, white_brush);
                    }
                    if (thumb_region != 0) {
                        DeleteObject(thumb_region);
                    }
                    text_rectangle.right = control_rect.left - 10;
                }
                SetTextColor(draw_item->hDC, win32_color(text_color));
                SetBkMode(draw_item->hDC, TRANSPARENT);
                DrawTextW(draw_item->hDC, button->label, -1, &text_rectangle,
                          DT_VCENTER | DT_SINGLELINE);
                if (button->is_checkbox && button->active) {
                    RECT mark_rectangle = control_rect;
                    SetTextColor(draw_item->hDC, win32_color(0xFFFFFFU));
                    DrawTextW(draw_item->hDC, L"X", 1, &mark_rectangle,
                              DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                }
                if (white_brush != 0) {
                    DeleteObject(white_brush);
                }
                if (frame_brush != 0) {
                    DeleteObject(frame_brush);
                }
                if (control_region != 0) {
                    DeleteObject(control_region);
                }
                if (fill_brush != 0) {
                    DeleteObject(fill_brush);
                }
                return 1;
            }
            int width = draw_item->rcItem.right - draw_item->rcItem.left;
            int height = draw_item->rcItem.bottom - draw_item->rcItem.top;
            int maximum_radius = width < height ? width / 2 : height / 2;
            int radius = clamp_dimension(button->corner_radius, 0, maximum_radius);
            BOOL disabled = button->disabled || (draw_item->itemState & ODS_DISABLED) != 0;
            BOOL pressed = !disabled && (button->pressed ||
                                         (draw_item->itemState & ODS_SELECTED) != 0);
            BOOL hovered = !disabled && (button->hovered ||
                                         (draw_item->itemState & ODS_HOTLIGHT) != 0);
            UINT fill_color = button->background_color;
            UINT text_color = button->text_color;
            HBRUSH fill_brush;
            HBRUSH focus_brush = 0;
            HBRUSH hover_brush = 0;
            HBRUSH outside_brush = button->flat_style ? button->brush : desktop.background_brush;
            HRGN region = radius > 0 ? CreateRoundRectRgn(
                draw_item->rcItem.left, draw_item->rcItem.top,
                draw_item->rcItem.right, draw_item->rcItem.bottom,
                radius * 2, radius * 2) : 0;
            RECT text_rectangle = draw_item->rcItem;
            if (disabled) {
                fill_color = adjust_rgb(fill_color, -45);
                text_color = adjust_rgb(text_color, -100);
            } else if (pressed) {
                fill_color = adjust_rgb(fill_color, -38);
                text_rectangle.left += 1;
                text_rectangle.top += 1;
                text_rectangle.right += 1;
                text_rectangle.bottom += 1;
            } else if (button->active) {
                fill_color = adjust_rgb(fill_color, 22);
            } else if (hovered) {
                fill_color = adjust_rgb(fill_color, 32);
            }
            fill_brush = CreateSolidBrush(win32_color(fill_color));
            if (fill_brush == 0) {
                fill_brush = button->brush;
            }
            if (hovered) {
                hover_brush = CreateSolidBrush(win32_color(adjust_rgb(button->background_color, 70)));
            }
            /* Owner-drawn BUTTON controls otherwise leave the default white
             * pixels outside a rounded region. Paint the whole control first
             * with the surface below it, then draw the rounded foreground. */
            FillRect(draw_item->hDC, &draw_item->rcItem, outside_brush);
            if (region != 0) {
                FillRgn(draw_item->hDC, region, fill_brush);
                if (!button->flat_style) {
                    FrameRgn(draw_item->hDC, region, desktop.border_brush, 1, 1);
                }
                if (hover_brush != 0) {
                    FrameRgn(draw_item->hDC, region, hover_brush, 2, 2);
                }
            } else {
                FillRect(draw_item->hDC, &draw_item->rcItem, fill_brush);
                if (!button->flat_style) {
                    FrameRect(draw_item->hDC, &draw_item->rcItem, desktop.border_brush);
                }
                if (hover_brush != 0) {
                    FrameRect(draw_item->hDC, &draw_item->rcItem, hover_brush);
                }
            }
            if (!disabled && (draw_item->itemState & ODS_FOCUS) != 0) {
                focus_brush = CreateSolidBrush(win32_color(0xFFFFFFU));
                if (focus_brush != 0) {
                    if (region != 0) {
                        FrameRgn(draw_item->hDC, region, focus_brush, 1, 1);
                    } else {
                        FrameRect(draw_item->hDC, &draw_item->rcItem, focus_brush);
                    }
                }
            }
            SetTextColor(draw_item->hDC, win32_color(text_color));
            SetBkMode(draw_item->hDC, TRANSPARENT);
            DrawTextW(draw_item->hDC, button->label, -1, &text_rectangle,
                      DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            if (focus_brush != 0) {
                DeleteObject(focus_brush);
            }
            if (hover_brush != 0) {
                DeleteObject(hover_brush);
            }
            if (region != 0) {
                DeleteObject(region);
            }
            if (fill_brush != button->brush) {
                DeleteObject(fill_brush);
            }
            return 1;
        }
    }
    if (message == WM_DESTROY) {
#if JADREN_UI_HAS_EVENT_CALLBACK
        if (desktop.events_ready && !desktop.close_event_sent &&
            desktop.close_event_id != 0) {
            desktop.close_event_sent = 1;
            sync_app_bindings_from_native();
            jadren_ui_on_click(desktop.close_event_id);
            refresh_app_bindings();
        }
#endif
        dispose_images();
        dispose_brushes();
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(window, message, wparam, lparam);
}

int jadren_win32_window_run(void) {
    static const wchar_t class_name[] = L"JadrenDesktopPreviewWindow";
    WNDCLASSEXW window_class = {0};
    INITCOMMONCONTROLSEX common_controls = {(DWORD)sizeof(INITCOMMONCONTROLSEX), ICC_WIN95_CLASSES};
    HINSTANCE instance = GetModuleHandleW(0);
    HWND window;
    MSG message;
    int index;

    if (instance == 0) {
        return startup_error(1, L"GetModuleHandleW failed.");
    }
    if (desktop.title[0] == L'\0') {
        jadren_win32_window_begin("Jadren Desktop", 14, 720, 430, 0xF6F8FCU);
    }
    if (InitCommonControlsEx(&common_controls) == 0) {
        return startup_error(15, L"InitCommonControlsEx tooltip support failed.");
    }
    active_theme_mode = normalize_theme_mode(declared_theme_mode);
    desktop.background_brush = CreateSolidBrush(win32_color(desktop.background_color));
    desktop.border_brush = CreateSolidBrush(win32_color(theme_color_for(active_theme_mode, UI_COLOR_BORDER)));
    if (desktop.background_brush == 0 || desktop.border_brush == 0) {
        return startup_error(2, L"CreateSolidBrush failed.");
    }

    window_class.cbSize = (UINT)sizeof(WNDCLASSEXW);
    window_class.style = CS_HREDRAW | CS_VREDRAW;
    window_class.lpfnWndProc = desktop_window_proc;
    window_class.hInstance = instance;
    window_class.hCursor = LoadCursorW(0, (LPCWSTR)(unsigned __int64)IDC_ARROW);
    window_class.hbrBackground = desktop.background_brush;
    window_class.lpszClassName = class_name;
    if (window_class.hCursor == 0) {
        return startup_error(3, L"LoadCursorW failed.");
    }
    if (RegisterClassExW(&window_class) == 0) {
        return startup_error(4, L"RegisterClassExW failed.");
    }

    /* Keep the parent hidden until every child panel and control exists. This
     * prevents the user from seeing a partially painted first frame. */
    window = CreateWindowExW(0, class_name, desktop.title, WS_OVERLAPPEDWINDOW,
                             (int)0x80000000, (int)0x80000000, desktop.width, desktop.height,
                             0, 0, instance, 0);
    if (window == 0) {
        return startup_error(5, L"CreateWindowExW failed.");
    }
    desktop.window = window;
    for (index = 0; index < desktop.label_count; index += 1) {
        LabelSpec *label = &desktop.labels[index];
        label->brush = CreateSolidBrush(win32_color(label->background_color));
        label->handle = CreateWindowExW(0, L"STATIC", label->text, WS_CHILD | WS_VISIBLE,
                                        label->x, label->y, label->width, label->height,
                                        window, 0, instance, 0);
        apply_corner_radius(label->handle, label->width, label->height, label->corner_radius);
    }
    for (index = 0; index < desktop.button_count; index += 1) {
        ButtonSpec *button = &desktop.buttons[index];
        button->brush = CreateSolidBrush(win32_color(button->background_color));
        button->handle = CreateWindowExW(0, L"BUTTON", button->label,
                                         WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW | BS_NOTIFY |
                                         (button->is_checkbox ? 0x0003 : 0) |
                                         (button->disabled ? WS_DISABLED : 0),
                                         button->x, button->y, button->width, button->height,
                                         window, (HMENU)(unsigned __int64)button->id, instance, 0);
        if (button->handle == 0) {
            return startup_error(6, L"CreateWindowExW button failed.");
        }
        if (button->is_checkbox) {
            SendMessageW(button->handle, 0x00F1,
                         (WPARAM)(button->active ? 1 : 0), 0);
        }
        if (button->system_icon) {
            button->font = CreateFontW(-18, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 0, 0,
                                       L"Segoe MDL2 Assets");
            if (button->font != 0) {
                SendMessageW(button->handle, 0x0030, (WPARAM)button->font, 1);
            }
        }
        button->default_proc = (WNDPROC)(unsigned __int64)SetWindowLongPtrW(
            button->handle, GWLP_WNDPROC, (LONG_PTR)(unsigned __int64)desktop_button_proc);
        if (button->default_proc == 0) {
            return startup_error(7, L"SetWindowLongPtrW button subclass failed.");
        }
    }
    for (index = 0; index < desktop.input_count; index += 1) {
        InputSpec *input = &desktop.inputs[index];
        input->brush = CreateSolidBrush(win32_color(input->background_color));
        input->handle = CreateWindowExW(0, L"EDIT", input->text,
                                        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER |
                                        ES_AUTOHSCROLL,
                                        input->x, input->y, input->width, input->height,
                                        window, (HMENU)(unsigned __int64)input->id, instance, 0);
        if (input->handle == 0) {
            return startup_error(8, L"CreateWindowExW text input failed.");
        }
        apply_corner_radius(input->handle, input->width, input->height, input->corner_radius);
        input->default_proc = (WNDPROC)(unsigned __int64)SetWindowLongPtrW(
            input->handle, GWLP_WNDPROC, (LONG_PTR)(unsigned __int64)desktop_input_proc);
        if (input->default_proc == 0) {
            return startup_error(10, L"SetWindowLongPtrW text input subclass failed.");
        }
    }
    for (index = 0; index < desktop.select_count; index += 1) {
        SelectSpec *select = &desktop.selects[index];
        int option_index;
        select->brush = CreateSolidBrush(win32_color(select->background_color));
        select->handle = CreateWindowExW(0, L"COMBOBOX", L"",
                                         WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL |
                                         CBS_DROPDOWNLIST | CBS_NOINTEGRALHEIGHT,
                                         select->x, select->y, select->width,
                                         select_window_height(select),
                                         window, (HMENU)(unsigned __int64)select->id, instance, 0);
        if (select->handle == 0) {
            return startup_error(9, L"CreateWindowExW select failed.");
        }
        apply_corner_radius(select->handle, select->width, select->height,
                            select->corner_radius);
        for (option_index = 0; option_index < select->option_count; option_index += 1) {
            SendMessageW(select->handle, CB_ADDSTRING, 0,
                         (LPARAM)(unsigned __int64)select->options[option_index]);
        }
        if (select->option_count > 0) {
            if (select->selected_index < 0 || select->selected_index >= select->option_count) {
                select->selected_index = 0;
            }
            SendMessageW(select->handle, CB_SETCURSEL,
                         (WPARAM)(unsigned __int64)select->selected_index, 0);
        }
        refresh_select_from_app_state(select);
    }
    for (index = 0; index < desktop.scroll_count; index += 1) {
        ScrollSpec *scroll = &desktop.scrolls[index];
        scroll->brush = CreateSolidBrush(win32_color(scroll->background_color));
        scroll->handle = CreateWindowExW(0, L"EDIT", scroll->text,
                                         WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | WS_VSCROLL |
                                         ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
                                         scroll->x, scroll->y, scroll->width, scroll->height,
                                         window, (HMENU)(unsigned __int64)scroll->id, instance, 0);
        if (scroll->handle == 0) {
            return startup_error(11, L"CreateWindowExW scroll panel failed.");
        }
        apply_corner_radius(scroll->handle, scroll->width, scroll->height, scroll->corner_radius);
    }
    for (index = 0; index < desktop.list_count; index += 1) {
        ListSpec *list = &desktop.lists[index];
        int item_index;
        list->brush = CreateSolidBrush(win32_color(list->background_color));
        list->handle = CreateWindowExW(0, L"LISTBOX", L"",
                                       WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | WS_VSCROLL |
                                       LBS_NOTIFY,
                                       list->x, list->y, list->width, list->height,
                                       window, (HMENU)(unsigned __int64)list->id, instance, 0);
        if (list->handle == 0) {
            return startup_error(12, L"CreateWindowExW list failed.");
        }
        apply_corner_radius(list->handle, list->width, list->height,
                            list->corner_radius);
        for (item_index = 0; item_index < list->item_count; item_index += 1) {
            SendMessageW(list->handle, LB_ADDSTRING, 0,
                         (LPARAM)(unsigned __int64)list->items[item_index]);
        }
        if (list->item_count > 0) {
            if (list->selected_index < 0 || list->selected_index >= list->item_count) {
                list->selected_index = 0;
            }
            SendMessageW(list->handle, LB_SETCURSEL,
                         (WPARAM)(unsigned __int64)list->selected_index, 0);
        }
        refresh_list_from_app_state(list);
    }
    for (index = 0; index < desktop.table_count; index += 1) {
        TableSpec *table = &desktop.tables[index];
        int column_index;
        table->brush = CreateSolidBrush(win32_color(table->background_color));
        table->handle = CreateWindowExW(0, L"SysListView32", L"",
                                        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER |
                                        WS_VSCROLL | WS_HSCROLL |
                                        LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS,
                                        table->x, table->y, table->width, table->height,
                                        window, (HMENU)(unsigned __int64)table->id, instance, 0);
        if (table->handle == 0) {
            return startup_error(18, L"CreateWindowExW table failed.");
        }
        apply_corner_radius(table->handle, table->width, table->height,
                            table->corner_radius);
        SendMessageW(table->handle, LVM_SETEXTENDEDLISTVIEWSTYLE,
                     LVS_EX_FULLROWSELECT, LVS_EX_FULLROWSELECT);
        SendMessageW(table->handle, LVM_SETBKCOLOR, 0,
                     (LPARAM)win32_color(table->background_color));
        SendMessageW(table->handle, LVM_SETTEXTCOLOR, 0,
                     (LPARAM)win32_color(table->text_color));
        SendMessageW(table->handle, LVM_SETTEXTBKCOLOR, 0,
                     (LPARAM)win32_color(table->background_color));
        for (column_index = 0; column_index < table->column_count; column_index += 1) {
            LVCOLUMNW column = {0};
            column.mask = LVCF_TEXT | LVCF_WIDTH;
            column.cx = table->header_widths[column_index];
            column.pszText = table->headers[column_index];
            SendMessageW(table->handle, LVM_INSERTCOLUMNW,
                         (WPARAM)(unsigned __int64)column_index, (LPARAM)&column);
        }
        refresh_table_control(table);
        refresh_table_from_app_state(table);
    }
    for (index = 0; index < desktop.tooltip_count; index += 1) {
        TooltipSpec *tooltip = &desktop.tooltips[index];
        ButtonSpec *tool = 0;
        TOOLINFOW tool_info = {0};
        int button_index;
        for (button_index = 0; button_index < desktop.button_count; button_index += 1) {
            ButtonSpec *candidate = &desktop.buttons[button_index];
            if (candidate->emits_event && candidate->event_id == tooltip->event_id) {
                tool = candidate;
                break;
            }
        }
        if (tool == 0) {
            return startup_error(16, L"ui_tooltip event_id has no event control.");
        }
        /* Native tooltips are popup windows, not child panels. Windows owns
         * their delay, z-order, placement and repaint, so they never dirty the
         * control tree below. The operating system chooses the natural height
         * and shape; width remains the Jadren wrapping limit. */
        tooltip->handle = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                                          L"tooltips_class32", 0, WS_POPUP | TTS_NOPREFIX,
                                          CW_USEDEFAULT, CW_USEDEFAULT,
                                          CW_USEDEFAULT, CW_USEDEFAULT,
                                          window, 0, instance, 0);
        if (tooltip->handle == 0) {
            return startup_error(13, L"CreateWindowExW tooltip failed.");
        }
        SetWindowPos(tooltip->handle, (HWND)(LONG_PTR)-1, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        SendMessageW(tooltip->handle, TTM_SETTIPBKCOLOR,
                     (WPARAM)win32_color(tooltip->background_color), 0);
        SendMessageW(tooltip->handle, TTM_SETTIPTEXTCOLOR,
                     (WPARAM)win32_color(tooltip->text_color), 0);
        SendMessageW(tooltip->handle, TTM_SETMAXTIPWIDTH, 0, (LPARAM)tooltip->width);
        /* V2 ends at lParam and is accepted by every supported common-controls
         * version; lpReserved is a later optional extension we do not use. */
        tool_info.cbSize = (UINT)(sizeof(TOOLINFOW) - sizeof(void *));
        tool_info.uFlags = TTF_IDISHWND | TTF_CENTERTIP;
        tool_info.hwnd = window;
        tool_info.uId = (UINT_PTR)tool->handle;
        tool_info.lpszText = tooltip->text;
        SendMessageW(tooltip->handle, TTM_ADDTOOLW, 0,
                     (LPARAM)(unsigned __int64)&tool_info);
        if (SendMessageW(tooltip->handle, TTM_GETTOOLCOUNT, 0, 0) == 0) {
            return startup_error(17, L"TTM_ADDTOOLW tooltip registration failed.");
        }
    }

    if (!load_images()) {
        return startup_error(14, L"PNG image asset failed to load.");
    }

    layout_controls(window);
    desktop.events_ready = 1;
    ShowWindow(window, SW_SHOW);
    UpdateWindow(window);
    /* The complete tree is now visible. Force its first paint before entering
     * the message loop so rounded panels are complete on the initial frame. */
    for (index = 0; index < desktop.label_count; index += 1) {
        UpdateWindow(desktop.labels[index].handle);
    }
    for (index = 0; index < desktop.button_count; index += 1) {
        UpdateWindow(desktop.buttons[index].handle);
    }
    for (index = 0; index < desktop.input_count; index += 1) {
        UpdateWindow(desktop.inputs[index].handle);
    }
    for (index = 0; index < desktop.select_count; index += 1) {
        UpdateWindow(desktop.selects[index].handle);
    }
    for (index = 0; index < desktop.scroll_count; index += 1) {
        UpdateWindow(desktop.scrolls[index].handle);
    }
    for (index = 0; index < desktop.list_count; index += 1) {
        UpdateWindow(desktop.lists[index].handle);
    }
    for (index = 0; index < desktop.table_count; index += 1) {
        UpdateWindow(desktop.tables[index].handle);
    }
    for (index = 0; index < desktop.tooltip_count; index += 1) {
        UpdateWindow(desktop.tooltips[index].handle);
    }
    RedrawWindow(window, 0, 0,
                 RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW);
    while (GetMessageW(&message, 0, 0, 0) > 0) {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    ExitProcess(0);
    return 0;
}

int ui_run(void) {
    return jadren_win32_window_run();
}

int ui_app_run(void) {
    /* An unclosed retained scope is a source-level layout error. Refuse to
     * enter the native message loop instead of rendering a partial tree. */
    if (portable_ui_active || portable_ui_depth != 0) {
        return 2;
    }
    return ui_run();
}

/* Compatibility entry point for the original two-string tutorial. */
int jadren_win32_desktop_run(const char *title_data, unsigned __int64 title_length,
                             const char *subtitle_data, unsigned __int64 subtitle_length) {
    jadren_win32_window_begin(title_data, title_length, 680, 390, 0xFFFFFFU);
    jadren_win32_label("Jadren Desktop Preview", 22, 28, 28, 560, 28, 0x111827U, 0xF1F5F9U);
    jadren_win32_label(subtitle_data, subtitle_length, 28, 70, 590, 50, 0x111827U, 0xF1F5F9U);
    jadren_win32_status("Pripravene na akciu.", 21, 28, 146, 590, 46, 0x111827U, 0xF1F5F9U);
    jadren_win32_button("Spusti akciu", 13,
                         "Akcia prebehla. Toto okno vytvoril Jadren cez Win32 runtime.", 66,
                         28, 236, 180, 42, 0x111827U, 0xE2E8F0U);
    jadren_win32_close_button("Zavriet", 7, 222, 236, 140, 42, 0x111827U, 0xE2E8F0U);
    return jadren_win32_window_run();
}
