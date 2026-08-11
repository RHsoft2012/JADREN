#include <X11/Xlib.h>
#include <X11/keysym.h>
#include <X11/Xutil.h>
#include <stdint.h>
#include <stddef.h>
#include <locale.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/*
 * Small native POSIX retained backend for the first Linux desktop slice.
 *
 * The Jadren source owns the retained tree.  This runtime only translates
 * that tree into bounded X11 primitives, so the platform-neutral jadren-ui
 * model remains independent of Xlib.  The deliberately small ABI mirrors
 * ui_app_* and keeps all storage bounded for freestanding generated programs.
 */

#define JADREN_X11_MAX_NODES 128
#define JADREN_X11_MAX_STACK 16
#define JADREN_X11_MAX_MENU_ITEMS 16
#define JADREN_X11_MAX_LIST_ITEMS 64
#define JADREN_X11_LIST_ROW_HEIGHT 28
#define JADREN_X11_MAX_TABLES 4
#define JADREN_X11_MAX_TABLE_ROWS 64
#define JADREN_X11_MAX_TABLE_COLUMNS 8
#define JADREN_X11_TABLE_HEADER_HEIGHT 28
#define JADREN_X11_TABLE_ROW_HEIGHT 28
#define JADREN_X11_MAX_TEXT 256
#define JADREN_X11_MAX_STATE_SLOTS 32
#define JADREN_X11_MAX_STATE_BINDINGS 64
#define JADREN_X11_MAX_INPUT_APP_BINDINGS 12

typedef struct JadrenString {
    const unsigned char *data;
    uint64_t length;
} JadrenString;

typedef enum JadrenNodeKind {
    JADREN_NODE_ROOT = 1,
    JADREN_NODE_PANEL = 2,
    JADREN_NODE_ROW = 3,
    JADREN_NODE_TOP_BAR = 4,
    JADREN_NODE_MENU = 5,
    JADREN_NODE_LABEL = 6,
    JADREN_NODE_STATUS = 7,
    JADREN_NODE_BUTTON = 8,
    JADREN_NODE_CHECKBOX = 9,
    JADREN_NODE_INPUT = 10,
    JADREN_NODE_SELECT = 11,
    JADREN_NODE_LIST = 12,
    JADREN_NODE_TABLE = 13
} JadrenNodeKind;

typedef struct JadrenNode {
    int used;
    int id;
    int kind;
    int parent;
    int width;
    int height;
    int padding;
    int gap;
    int stretch;
    int event_id;
    int checked;
    uint32_t text_color;
    uint32_t background_color;
    int x;
    int y;
    int laid_width;
    int laid_height;
    char text[JADREN_X11_MAX_TEXT];
    int tooltip_defined;
    int tooltip_width;
    int tooltip_height;
    uint32_t tooltip_text_color;
    uint32_t tooltip_background_color;
    int tooltip_corner_radius;
    char tooltip_text[JADREN_X11_MAX_TEXT];
    int menu_item_count;
    int menu_item_events[JADREN_X11_MAX_MENU_ITEMS];
    char menu_item_text[JADREN_X11_MAX_MENU_ITEMS][JADREN_X11_MAX_TEXT];
    int option_count;
    int selected_index;
    char option_text[JADREN_X11_MAX_MENU_ITEMS][JADREN_X11_MAX_TEXT];
    int list_item_count;
    int list_selected_index;
    int app_list_id;
    char list_item_text[JADREN_X11_MAX_LIST_ITEMS][JADREN_X11_MAX_TEXT];
    int table_slot;
} JadrenNode;

typedef struct JadrenTableData {
    int used;
    int column_count;
    int row_count;
    int selected_row;
    int app_table_id;
    int app_table_column_count;
    int column_widths[JADREN_X11_MAX_TABLE_COLUMNS];
    char headers[JADREN_X11_MAX_TABLE_COLUMNS][JADREN_X11_MAX_TEXT];
    char cells[JADREN_X11_MAX_TABLE_ROWS][JADREN_X11_MAX_TABLE_COLUMNS]
              [JADREN_X11_MAX_TEXT];
} JadrenTableData;

typedef struct JadrenStateBinding {
    int event_id;
    int slot;
    int mode;
} JadrenStateBinding;

typedef struct JadrenInputAppBinding {
    int event_id;
    size_t key_length;
    char key[65];
    int kind;
} JadrenInputAppBinding;

enum {
    JADREN_APP_BIND_TEXT = 1,
    JADREN_APP_BIND_BOOL = 2,
    JADREN_APP_BIND_SELECT = 3,
    JADREN_APP_BIND_LIST = 4,
    JADREN_APP_BIND_TABLE = 5
};

static JadrenNode jadren_nodes[JADREN_X11_MAX_NODES];
static int jadren_next_node;
static int jadren_stack[JADREN_X11_MAX_STACK];
static int jadren_stack_depth;
static int jadren_active;
static int jadren_window_width;
static int jadren_window_height;
static int jadren_window_min_width;
static int jadren_window_min_height;
static int jadren_window_max_width;
static int jadren_window_max_height;
static uint32_t jadren_window_background;
static char jadren_window_title[JADREN_X11_MAX_TEXT];
static int jadren_status_node;
static int jadren_hover_node;
static int jadren_focused_node;
static int jadren_open_menu;
static int jadren_open_select;
static int jadren_last_event;
static int jadren_resize_event_id;
static int jadren_close_event_id;
static int jadren_close_event_sent;
static JadrenTableData jadren_tables[JADREN_X11_MAX_TABLES];
static int jadren_state_slots[JADREN_X11_MAX_STATE_SLOTS];
static char jadren_state_text[JADREN_X11_MAX_STATE_SLOTS][JADREN_X11_MAX_TEXT];
static JadrenStateBinding
    jadren_state_bindings[JADREN_X11_MAX_STATE_BINDINGS];
static int jadren_state_binding_count;
static JadrenInputAppBinding
    jadren_input_app_bindings[JADREN_X11_MAX_INPUT_APP_BINDINGS];
static int jadren_input_app_binding_count;

static Display *jadren_display;
static int jadren_screen;
static Window jadren_window;
static GC jadren_gc;
static Atom jadren_delete_atom;
/* XIM is optional.  When the active locale/input method cannot be opened,
 * the backend keeps the historical XLookupString ASCII fallback instead of
 * making the whole retained window fail to start. */
static XIM jadren_input_method;
static XIC jadren_input_context;

/* The file runtime owns app_list/app_table storage. Weak references keep a
 * local-only X11 list/table linkable without pulling the file runtime into
 * every UI program. */
extern int app_list_count(int list_id) __attribute__((weak));
extern uint64_t app_list_read_text(int list_id, int item_index,
                                   unsigned char *output_data,
                                   uint64_t output_length)
    __attribute__((weak));
extern int app_table_row_count(int table_id) __attribute__((weak));
extern uint64_t app_table_read_cell(int table_id, int row_index,
                                    int column_index,
                                    unsigned char *output_data,
                                    uint64_t output_length)
    __attribute__((weak));
extern int app_table_sort_text(int table_id, int column_index, int descending)
    __attribute__((weak));
extern int app_table_sort_int(int table_id, int column_index, int descending)
    __attribute__((weak));
extern int app_table_sort_uint(int table_id, int column_index, int descending)
    __attribute__((weak));
extern int app_table_sort_float(int table_id, int column_index, int descending)
    __attribute__((weak));
extern int app_table_sort_bool(int table_id, int column_index, int descending)
    __attribute__((weak));
extern int app_table_filter_text(int source_table_id, int destination_table_id,
                                 int column_index, const char *query_data,
                                 uint64_t query_length)
    __attribute__((weak));
extern int app_table_filter_text_ex(int source_table_id, int destination_table_id,
                                    int column_index, const char *query_data,
                                    uint64_t query_length, int mode)
    __attribute__((weak));
extern int app_table_filter_int(int source_table_id, int destination_table_id,
                                int column_index, int64_t query)
    __attribute__((weak));
extern int app_table_filter_uint(int source_table_id, int destination_table_id,
                                 int column_index, uint64_t query)
    __attribute__((weak));
extern int app_table_filter_float(int source_table_id, int destination_table_id,
                                  int column_index, double query)
    __attribute__((weak));
extern int app_table_filter_bool(int source_table_id, int destination_table_id,
                                 int column_index, unsigned char query)
    __attribute__((weak));

#if JADREN_UI_HAS_FILE_RUNTIME
extern int app_state_set_text(const char *key_data, uint64_t key_length,
                              const char *value_data, uint64_t value_length);
extern uint64_t app_state_read_text(const char *key_data, uint64_t key_length,
                                    unsigned char *output_data,
                                    uint64_t output_length);
extern int app_state_set_bool(const char *key_data, uint64_t key_length,
                              unsigned char value);
extern int app_state_get_bool(const char *key_data, uint64_t key_length);
extern int app_state_set_int(const char *key_data, uint64_t key_length,
                             int64_t value);
extern int64_t app_state_get_int(const char *key_data, uint64_t key_length);
#endif

#if JADREN_UI_HAS_EVENT_CALLBACK
extern int32_t jadren_ui_on_click(int32_t event_id);
#endif

extern int32_t jadren_entry(void);

#if JADREN_UI_HAS_FILE_RUNTIME
extern void jadren_process_args_init(int argc, char **argv);
#endif

static void jadren_sync_state_bindings_for_event(int event_id);
static void jadren_sync_input_app_state(JadrenNode *input);
static void jadren_refresh_input_from_app_state(JadrenNode *input);
static void jadren_sync_checkbox_app_state(JadrenNode *checkbox);
static void jadren_refresh_checkbox_from_app_state(JadrenNode *checkbox);
static void jadren_sync_select_app_state(JadrenNode *select);
static void jadren_refresh_select_from_app_state(JadrenNode *select);
static void jadren_sync_list_app_state(JadrenNode *list);
static void jadren_refresh_list_from_app_state(JadrenNode *list);
static void jadren_sync_table_app_state(JadrenNode *table);
static void jadren_refresh_table_from_app_state(JadrenNode *table);
static void jadren_sync_app_bindings_from_native(void);
void ui_app_refresh_app_state(int node_id);
/* Retained node binding may attach to an already-loaded model. */
static int jadren_preserve_app_binding;
static JadrenNode *jadren_input_for_event(int event_id);
static JadrenInputAppBinding *jadren_input_app_binding_for_event(int event_id);
static JadrenNode *jadren_node_for_event(int event_id, int kind);
void ui_refresh_bindings(void);

static void jadren_copy_text(char *destination, size_t capacity,
                             const char *data, uint64_t length) {
    size_t count;
    if (capacity == 0U) {
        return;
    }
    count = length > (uint64_t)(capacity - 1U)
                ? capacity - 1U
                : (size_t)length;
    if (data != NULL && count > 0U) {
        memcpy(destination, data, count);
    }
    destination[count] = '\0';
}

static JadrenNode *jadren_node(int id) {
    if (id <= 0 || id >= JADREN_X11_MAX_NODES || !jadren_nodes[id].used) {
        return NULL;
    }
    return &jadren_nodes[id];
}

static JadrenTableData *jadren_table_data(const JadrenNode *node) {
    if (node == NULL || node->kind != JADREN_NODE_TABLE ||
        node->table_slot < 0 || node->table_slot >= JADREN_X11_MAX_TABLES ||
        !jadren_tables[node->table_slot].used) {
        return NULL;
    }
    return &jadren_tables[node->table_slot];
}

static int jadren_free_table_slot(void) {
    int index;
    for (index = 0; index < JADREN_X11_MAX_TABLES; index += 1) {
        if (!jadren_tables[index].used) {
            return index;
        }
    }
    return -1;
}

static int jadren_current_parent(int parent) {
    return jadren_active && jadren_stack_depth > 0 &&
           jadren_stack[jadren_stack_depth - 1] == parent;
}

static int jadren_allocate_node(int kind, int parent) {
    JadrenNode *node;
    int id;
    if (!jadren_current_parent(parent) || jadren_next_node >= JADREN_X11_MAX_NODES) {
        return 0;
    }
    id = jadren_next_node;
    jadren_next_node += 1;
    node = &jadren_nodes[id];
    memset(node, 0, sizeof(*node));
    node->used = 1;
    node->id = id;
    node->kind = kind;
    node->parent = parent;
    return id;
}

static int jadren_add_container(int kind, int parent, int width, int height,
                                int padding, int gap, int stretch,
                                uint32_t background_color) {
    int id = jadren_allocate_node(kind, parent);
    JadrenNode *node;
    if (id == 0 || jadren_stack_depth >= JADREN_X11_MAX_STACK) {
        return 0;
    }
    node = &jadren_nodes[id];
    node->width = width;
    node->height = height;
    node->padding = padding < 0 ? 0 : padding;
    node->gap = gap < 0 ? 0 : gap;
    node->stretch = stretch != 0;
    node->background_color = background_color;
    jadren_stack[jadren_stack_depth] = id;
    jadren_stack_depth += 1;
    return id;
}

static int jadren_add_leaf(int kind, int parent, const char *text_data,
                           uint64_t text_length, int event_id, int width,
                           int height, uint32_t text_color,
                           uint32_t background_color, int stretch) {
    int id = jadren_allocate_node(kind, parent);
    JadrenNode *node;
    if (id == 0) {
        return 0;
    }
    node = &jadren_nodes[id];
    node->width = width;
    node->height = height;
    node->event_id = event_id;
    node->text_color = text_color;
    node->background_color = background_color;
    node->stretch = stretch != 0;
    jadren_copy_text(node->text, sizeof(node->text), text_data, text_length);
    if (kind == JADREN_NODE_STATUS) {
        jadren_status_node = id;
    }
    return id;
}

static unsigned long jadren_pixel(uint32_t color) {
    XColor value;
    value.red = (unsigned short)(((color >> 16) & 0xffU) * 257U);
    value.green = (unsigned short)(((color >> 8) & 0xffU) * 257U);
    value.blue = (unsigned short)((color & 0xffU) * 257U);
    value.flags = DoRed | DoGreen | DoBlue;
    if (XAllocColor(jadren_display, DefaultColormap(jadren_display, jadren_screen),
                   &value) == 0) {
        return BlackPixel(jadren_display, jadren_screen);
    }
    return value.pixel;
}

static void jadren_layout_node(int id, int x, int y, int width, int height) {
    JadrenNode *node = jadren_node(id);
    int child_ids[JADREN_X11_MAX_NODES];
    int child_count = 0;
    int index;
    int cursor;
    int inner_x;
    int inner_y;
    int inner_width;
    int inner_height;
    int horizontal;
    if (node == NULL) {
        return;
    }
    node->x = x;
    node->y = y;
    node->laid_width = width < 1 ? 1 : width;
    node->laid_height = height < 1 ? 1 : height;
    for (index = 1; index < jadren_next_node; index += 1) {
        if (jadren_nodes[index].used && jadren_nodes[index].parent == id) {
            child_ids[child_count] = index;
            child_count += 1;
            if (child_count >= JADREN_X11_MAX_NODES) {
                break;
            }
        }
    }
    horizontal = node->kind == JADREN_NODE_ROW || node->kind == JADREN_NODE_TOP_BAR;
    inner_x = x + node->padding;
    inner_y = y + node->padding;
    inner_width = width - 2 * node->padding;
    inner_height = height - 2 * node->padding;
    if (inner_width < 1) {
        inner_width = 1;
    }
    if (inner_height < 1) {
        inner_height = 1;
    }
    cursor = horizontal ? inner_x : inner_y;
    for (index = 0; index < child_count; index += 1) {
        JadrenNode *child = &jadren_nodes[child_ids[index]];
        int child_width = child->width > 0 ? child->width : inner_width;
        int child_height = child->height > 0 ? child->height : 32;
        int child_x = horizontal ? cursor : inner_x;
        int child_y = horizontal ? inner_y : cursor;
        if (child->stretch) {
            if (horizontal) {
                child_height = inner_height;
            } else {
                child_width = inner_width;
            }
        }
        if (horizontal && child_x + child_width > inner_x + inner_width) {
            child_width = inner_x + inner_width - child_x;
        }
        if (!horizontal && child_y + child_height > inner_y + inner_height) {
            child_height = inner_y + inner_height - child_y;
        }
        if (child_width < 1) {
            child_width = 1;
        }
        if (child_height < 1) {
            child_height = 1;
        }
        jadren_layout_node(child->id, child_x, child_y, child_width, child_height);
        cursor += (horizontal ? child_width : child_height) + node->gap;
    }
}

static void jadren_draw_text(JadrenNode *node, int x, int y) {
    if (node == NULL || node->text[0] == '\0') {
        return;
    }
    XSetForeground(jadren_display, jadren_gc, jadren_pixel(node->text_color));
    XDrawString(jadren_display, jadren_window, jadren_gc, x + 10, y + 20,
                node->text, (int)strlen(node->text));
}

static void jadren_fill_node(JadrenNode *node, uint32_t color) {
    if (node == NULL) {
        return;
    }
    XSetForeground(jadren_display, jadren_gc, jadren_pixel(color));
    XFillRectangle(jadren_display, jadren_window, jadren_gc,
                   node->x, node->y, (unsigned int)node->laid_width,
                   (unsigned int)node->laid_height);
}

static void jadren_draw_node(int id) {
    JadrenNode *node = jadren_node(id);
    int index;
    if (node == NULL) {
        return;
    }
    if (node->kind == JADREN_NODE_ROOT || node->kind == JADREN_NODE_PANEL ||
        node->kind == JADREN_NODE_TOP_BAR) {
        jadren_fill_node(node, node->background_color);
    } else if (node->kind == JADREN_NODE_LIST) {
        jadren_fill_node(node, node->background_color);
        for (index = 0; index < node->list_item_count; index += 1) {
            int item_y = node->y + index * JADREN_X11_LIST_ROW_HEIGHT;
            int remaining_height = node->y + node->laid_height - item_y;
            if (item_y >= node->y + node->laid_height) {
                break;
            }
            if (index == node->list_selected_index) {
                XSetForeground(jadren_display, jadren_gc,
                               jadren_pixel(0xDCEBFFU));
                XFillRectangle(jadren_display, jadren_window, jadren_gc,
                               node->x, item_y,
                               (unsigned int)node->laid_width,
                               (unsigned int)(remaining_height <
                                                       JADREN_X11_LIST_ROW_HEIGHT
                                                   ? remaining_height
                                                   : JADREN_X11_LIST_ROW_HEIGHT));
            }
            XSetForeground(jadren_display, jadren_gc,
                           jadren_pixel(node->text_color));
            XDrawString(jadren_display, jadren_window, jadren_gc,
                        node->x + 10, item_y + 19,
                        node->list_item_text[index],
                        (int)strlen(node->list_item_text[index]));
        }
    } else if (node->kind == JADREN_NODE_TABLE) {
        JadrenTableData *table = jadren_table_data(node);
        int column_index;
        int row_index;
        int column_x = node->x;
        jadren_fill_node(node, node->background_color);
        if (table != NULL) {
            for (column_index = 0; column_index < table->column_count;
                 column_index += 1) {
                int column_width = table->column_widths[column_index];
                int remaining_width = node->x + node->laid_width - column_x;
                if (remaining_width <= 0) {
                    break;
                }
                if (column_width > remaining_width) {
                    column_width = remaining_width;
                }
                XSetForeground(jadren_display, jadren_gc,
                               jadren_pixel(0xE5E7EBU));
                XFillRectangle(jadren_display, jadren_window, jadren_gc,
                               column_x, node->y,
                               (unsigned int)column_width,
                               JADREN_X11_TABLE_HEADER_HEIGHT);
                XSetForeground(jadren_display, jadren_gc,
                               jadren_pixel(node->text_color));
                XDrawString(jadren_display, jadren_window, jadren_gc,
                            column_x + 8, node->y + 19,
                            table->headers[column_index],
                            (int)strlen(table->headers[column_index]));
                column_x += column_width;
            }
            for (row_index = 0; row_index < table->row_count; row_index += 1) {
                int item_y = node->y + JADREN_X11_TABLE_HEADER_HEIGHT +
                             row_index * JADREN_X11_TABLE_ROW_HEIGHT;
                int remaining_height = node->y + node->laid_height - item_y;
                if (item_y >= node->y + node->laid_height) {
                    break;
                }
                if (row_index == table->selected_row) {
                    XSetForeground(jadren_display, jadren_gc,
                                   jadren_pixel(0xDCEBFFU));
                    XFillRectangle(
                        jadren_display, jadren_window, jadren_gc, node->x,
                        item_y, (unsigned int)node->laid_width,
                        (unsigned int)(remaining_height <
                                               JADREN_X11_TABLE_ROW_HEIGHT
                                           ? remaining_height
                                           : JADREN_X11_TABLE_ROW_HEIGHT));
                }
                column_x = node->x;
                for (column_index = 0;
                     column_index < table->column_count; column_index += 1) {
                    int column_width = table->column_widths[column_index];
                    int remaining_width = node->x + node->laid_width - column_x;
                    if (remaining_width <= 0) {
                        break;
                    }
                    if (column_width > remaining_width) {
                        column_width = remaining_width;
                    }
                    XSetForeground(jadren_display, jadren_gc,
                                   jadren_pixel(node->text_color));
                    XDrawString(jadren_display, jadren_window, jadren_gc,
                                column_x + 8, item_y + 19,
                                table->cells[row_index][column_index],
                                (int)strlen(
                                    table->cells[row_index][column_index]));
                    column_x += column_width;
                }
            }
        }
    } else if (node->kind != JADREN_NODE_ROW) {
        uint32_t color = node->background_color;
        if (node->id == jadren_hover_node && node->kind == JADREN_NODE_BUTTON) {
            color = color ^ 0x181818U;
        }
        jadren_fill_node(node, color);
        jadren_draw_text(node, node->x, node->y);
        if (node->kind == JADREN_NODE_INPUT && node->id == jadren_focused_node) {
            XSetForeground(jadren_display, jadren_gc, jadren_pixel(node->text_color));
            XDrawRectangle(jadren_display, jadren_window, jadren_gc,
                           node->x, node->y,
                           (unsigned int)(node->laid_width > 1 ? node->laid_width - 1 : 1),
                           (unsigned int)(node->laid_height > 1 ? node->laid_height - 1 : 1));
        }
    }
    for (index = 1; index < jadren_next_node; index += 1) {
        if (jadren_nodes[index].used && jadren_nodes[index].parent == id) {
            jadren_draw_node(index);
        }
    }
}

static void jadren_draw_tooltip(void) {
    JadrenNode *node = jadren_node(jadren_hover_node);
    int width;
    int height;
    int x;
    int y;
    int text_length;
    int max_text_length;
    if (node == NULL || !node->tooltip_defined) {
        return;
    }
    text_length = (int)strlen(node->tooltip_text);
    max_text_length = node->tooltip_width > 20 ?
                      (node->tooltip_width - 20) / 8 : 1;
    if (text_length > max_text_length) {
        text_length = max_text_length;
    }
    width = node->tooltip_width;
    if (width < 80) {
        width = 80;
    }
    if (width > jadren_window_width - 8) {
        width = jadren_window_width - 8;
    }
    height = node->tooltip_height;
    if (height < 22) {
        height = 22;
    }
    if (height > 120) {
        height = 120;
    }
    x = node->x;
    if (x + width > jadren_window_width - 4) {
        x = jadren_window_width - width - 4;
    }
    if (x < 4) {
        x = 4;
    }
    y = node->y - height - 6;
    if (y < 4) {
        y = node->y + node->laid_height + 6;
    }
    if (y + height > jadren_window_height - 4) {
        y = jadren_window_height - height - 4;
    }
    XSetForeground(jadren_display, jadren_gc,
                   jadren_pixel(node->tooltip_background_color));
    XFillRectangle(jadren_display, jadren_window, jadren_gc,
                   x, y, (unsigned int)width, (unsigned int)height);
    XSetForeground(jadren_display, jadren_gc,
                   jadren_pixel(node->tooltip_text_color));
    if (text_length > 0) {
        XDrawString(jadren_display, jadren_window, jadren_gc,
                    x + 10, y + 20, node->tooltip_text, text_length);
    }
}

static int jadren_point_in_node(const JadrenNode *node, int x, int y) {
    return node != NULL && x >= node->x && y >= node->y &&
           x < node->x + node->laid_width && y < node->y + node->laid_height;
}

static int jadren_list_row_at(const JadrenNode *list, int x, int y) {
    int row;
    if (list == NULL || list->kind != JADREN_NODE_LIST ||
        !jadren_point_in_node(list, x, y)) {
        return -1;
    }
    row = (y - list->y) / JADREN_X11_LIST_ROW_HEIGHT;
    return row >= 0 && row < list->list_item_count ? row : -1;
}

static int jadren_table_row_at(const JadrenNode *table_node, int x, int y) {
    JadrenTableData *table = jadren_table_data(table_node);
    int row;
    if (table == NULL || !jadren_point_in_node(table_node, x, y) ||
        y < table_node->y + JADREN_X11_TABLE_HEADER_HEIGHT) {
        return -1;
    }
    row = (y - table_node->y - JADREN_X11_TABLE_HEADER_HEIGHT) /
          JADREN_X11_TABLE_ROW_HEIGHT;
    return row >= 0 && row < table->row_count ? row : -1;
}

static int jadren_hit_test(int x, int y) {
    int index;
    if (jadren_open_select > 0) {
        JadrenNode *select = jadren_node(jadren_open_select);
        if (select != NULL) {
            for (index = 0; index < select->option_count; index += 1) {
                int item_y = select->y + select->laid_height + index * 28;
                if (x >= select->x && x < select->x + select->laid_width &&
                    y >= item_y && y < item_y + 28) {
                    return -(101 + index);
                }
            }
        }
        jadren_open_select = 0;
    }
    if (jadren_open_menu > 0) {
        JadrenNode *menu = jadren_node(jadren_open_menu);
        if (menu != NULL) {
            for (index = 0; index < menu->menu_item_count; index += 1) {
                int item_y = menu->y + menu->laid_height + index * 28;
                if (x >= menu->x && x < menu->x + menu->laid_width &&
                    y >= item_y && y < item_y + 28) {
                    return -(index + 1);
                }
            }
        }
        jadren_open_menu = 0;
    }
    for (index = jadren_next_node - 1; index >= 1; index -= 1) {
        JadrenNode *node = &jadren_nodes[index];
        if (node->used && (node->kind == JADREN_NODE_BUTTON ||
                           node->kind == JADREN_NODE_MENU ||
                           node->kind == JADREN_NODE_CHECKBOX ||
                           node->kind == JADREN_NODE_INPUT ||
                           node->kind == JADREN_NODE_SELECT ||
                           node->kind == JADREN_NODE_LIST ||
                           node->kind == JADREN_NODE_TABLE) &&
            jadren_point_in_node(node, x, y)) {
            return index;
        }
    }
    return 0;
}

static void jadren_draw(void) {
    int index;
    XClearWindow(jadren_display, jadren_window);
    jadren_layout_node(1, 16, 16, jadren_window_width - 32,
                       jadren_window_height - 32);
    jadren_draw_node(1);
    if (jadren_open_menu > 0) {
        JadrenNode *menu = jadren_node(jadren_open_menu);
        if (menu != NULL) {
            int menu_height = menu->menu_item_count * 28;
            XSetForeground(jadren_display, jadren_gc,
                           jadren_pixel(menu->background_color));
            XFillRectangle(jadren_display, jadren_window, jadren_gc,
                           menu->x, menu->y + menu->laid_height,
                           (unsigned int)menu->laid_width,
                           (unsigned int)menu_height);
            for (index = 0; index < menu->menu_item_count; index += 1) {
                XSetForeground(jadren_display, jadren_gc,
                               jadren_pixel(menu->text_color));
                XDrawString(jadren_display, jadren_window, jadren_gc,
                            menu->x + 10,
                            menu->y + menu->laid_height + index * 28 + 19,
                            menu->menu_item_text[index],
                            (int)strlen(menu->menu_item_text[index]));
            }
        }
    }
    if (jadren_open_select > 0) {
        JadrenNode *select = jadren_node(jadren_open_select);
        if (select != NULL) {
            int option_height = select->option_count * 28;
            XSetForeground(jadren_display, jadren_gc,
                           jadren_pixel(select->background_color));
            XFillRectangle(jadren_display, jadren_window, jadren_gc,
                           select->x, select->y + select->laid_height,
                           (unsigned int)select->laid_width,
                           (unsigned int)option_height);
            for (index = 0; index < select->option_count; index += 1) {
                XSetForeground(jadren_display, jadren_gc,
                               jadren_pixel(select->text_color));
                XDrawString(jadren_display, jadren_window, jadren_gc,
                            select->x + 10,
                            select->y + select->laid_height + index * 28 + 19,
                            select->option_text[index],
                            (int)strlen(select->option_text[index]));
            }
        }
    }
    jadren_draw_tooltip();
    XFlush(jadren_display);
}

static void jadren_dispatch_event(int event_id) {
    jadren_last_event = event_id;
    jadren_sync_app_bindings_from_native();
    jadren_sync_input_app_state(jadren_input_for_event(event_id));
    jadren_sync_state_bindings_for_event(event_id);
#if JADREN_UI_HAS_EVENT_CALLBACK
    if (event_id != 0) {
        (void)jadren_ui_on_click(event_id);
    }
#else
    (void)event_id;
#endif
    ui_refresh_bindings();
}

static uint64_t jadren_read_stream(int fd, unsigned char *output_data,
                                   uint64_t output_length) {
    if (output_data == NULL || output_length == 0U) {
        return 0U;
    }
    size_t chunk = output_length > (uint64_t)SIZE_MAX
        ? SIZE_MAX
        : (size_t)output_length;
    ssize_t received = read(fd, output_data, chunk);
    return received > 0 ? (uint64_t)received : 0U;
}

static uint64_t jadren_write_stream(int fd, const unsigned char *input_data,
                                    uint64_t input_length) {
    if (input_data == NULL && input_length > 0U) {
        return 0U;
    }
    const unsigned char *cursor = input_data;
    uint64_t remaining = input_length;
    uint64_t total = 0U;
    while (remaining > 0U) {
        size_t chunk = remaining > (uint64_t)SIZE_MAX
            ? SIZE_MAX
            : (size_t)remaining;
        ssize_t written = write(fd, cursor, chunk);
        if (written <= 0) {
            break;
        }
        cursor += (size_t)written;
        remaining -= (uint64_t)written;
        total += (uint64_t)written;
    }
    return total;
}

uint64_t stdin_read(unsigned char *output_data, uint64_t output_length) {
    return jadren_read_stream(STDIN_FILENO, output_data, output_length);
}

uint64_t stdout_write(const unsigned char *input_data, uint64_t input_length) {
    return jadren_write_stream(STDOUT_FILENO, input_data, input_length);
}

uint64_t stderr_write(const unsigned char *input_data, uint64_t input_length) {
    return jadren_write_stream(STDERR_FILENO, input_data, input_length);
}

void print(JadrenString value) {
    if (value.data != NULL && value.length > 0U) {
        uint64_t remaining = value.length;
        const unsigned char *cursor = value.data;
        while (remaining > 0U) {
            size_t chunk = remaining > (uint64_t)SIZE_MAX
                ? SIZE_MAX
                : (size_t)remaining;
            ssize_t written = write(STDOUT_FILENO, cursor, chunk);
            if (written <= 0) {
                break;
            }
            cursor += (size_t)written;
            remaining -= (uint64_t)written;
        }
    }
    static const unsigned char newline = '\n';
    (void)write(STDOUT_FILENO, &newline, 1U);
}

int32_t ui_app_begin(const char *title_data, uint64_t title_length,
                     int width, int height, uint32_t background_color) {
    int index;
    JadrenNode *root;
    if (jadren_active) {
        return 0;
    }
    memset(jadren_nodes, 0, sizeof(jadren_nodes));
    memset(jadren_tables, 0, sizeof(jadren_tables));
    memset(jadren_state_slots, 0, sizeof(jadren_state_slots));
    memset(jadren_state_text, 0, sizeof(jadren_state_text));
    memset(jadren_state_bindings, 0, sizeof(jadren_state_bindings));
    memset(jadren_input_app_bindings, 0, sizeof(jadren_input_app_bindings));
    jadren_state_binding_count = 0;
    jadren_input_app_binding_count = 0;
    jadren_next_node = 2;
    jadren_stack_depth = 0;
    jadren_active = 1;
    jadren_window_width = width > 160 ? width : 640;
    jadren_window_height = height > 120 ? height : 420;
    jadren_window_min_width = 160;
    jadren_window_min_height = 120;
    jadren_window_max_width = 8192;
    jadren_window_max_height = 8192;
    jadren_window_background = background_color;
    jadren_status_node = 0;
    jadren_hover_node = 0;
    jadren_focused_node = 0;
    jadren_open_menu = 0;
    jadren_open_select = 0;
    jadren_last_event = 0;
    jadren_resize_event_id = 0;
    jadren_close_event_id = 0;
    jadren_close_event_sent = 0;
    jadren_copy_text(jadren_window_title, sizeof(jadren_window_title),
                     title_data, title_length);
    for (index = 0; index < JADREN_X11_MAX_NODES; index += 1) {
        jadren_nodes[index].used = 0;
    }
    root = &jadren_nodes[1];
    memset(root, 0, sizeof(*root));
    root->used = 1;
    root->id = 1;
    root->kind = JADREN_NODE_ROOT;
    root->width = jadren_window_width;
    root->height = jadren_window_height;
    root->background_color = background_color;
    jadren_stack[0] = 1;
    jadren_stack_depth = 1;
    return 1;
}

/* Window lifecycle uses the same explicit callback/event-reducer boundary as
 * controls. Geometry is kept in the backend and read by Jadren at callback
 * time; no hidden event queue or platform handle crosses the API. */
void ui_app_on_resize(int event_id) {
    jadren_resize_event_id = event_id;
}

void ui_app_on_close(int event_id) {
    jadren_close_event_id = event_id;
}

int32_t ui_app_window_width(void) {
    return jadren_window_width;
}

int32_t ui_app_window_height(void) {
    return jadren_window_height;
}

int32_t ui_app_window_set_constraints(int min_width, int min_height,
                                      int max_width, int max_height) {
    if (min_width < 120 || min_height < 80 || max_width < min_width ||
        max_height < min_height || max_width > 8192 || max_height > 8192) {
        return 0;
    }
    jadren_window_min_width = min_width;
    jadren_window_min_height = min_height;
    jadren_window_max_width = max_width;
    jadren_window_max_height = max_height;
    return 1;
}

int32_t ui_app_window_min_width(void) {
    return jadren_window_min_width;
}

int32_t ui_app_window_min_height(void) {
    return jadren_window_min_height;
}

int32_t ui_app_window_max_width(void) {
    return jadren_window_max_width;
}

int32_t ui_app_window_max_height(void) {
    return jadren_window_max_height;
}

int32_t ui_app_panel(int parent, int width, int height, uint32_t background_color,
                     int corner_radius, int padding, int gap, int align,
                     int stretch) {
    (void)corner_radius;
    (void)align;
    return jadren_add_container(JADREN_NODE_PANEL, parent, width, height,
                                padding, gap, stretch, background_color);
}

int32_t ui_app_row(int parent, int width, int height, int padding, int gap,
                   int align, int stretch) {
    (void)align;
    return jadren_add_container(JADREN_NODE_ROW, parent, width, height,
                                padding, gap, stretch, 0xF6F8FCU);
}

int32_t ui_app_top_bar(int parent, int width, int height, uint32_t background_color,
                       int corner_radius, int padding, int gap, int align,
                       int stretch) {
    (void)corner_radius;
    (void)align;
    return jadren_add_container(JADREN_NODE_TOP_BAR, parent, width, height,
                                padding, gap, stretch, background_color);
}

int32_t ui_app_menu(int parent, const char *label_data, uint64_t label_length,
                    int width, int height, uint32_t text_color,
                    uint32_t background_color, int corner_radius, int stretch) {
    int id;
    JadrenNode *node;
    (void)corner_radius;
    id = jadren_add_leaf(JADREN_NODE_MENU, parent, label_data, label_length,
                         0, width, height, text_color, background_color, stretch);
    node = jadren_node(id);
    if (node != NULL) {
        node->menu_item_count = 0;
    }
    return id;
}

int32_t ui_app_menu_item(int menu_node, const char *label_data,
                         uint64_t label_length, int event_id) {
    JadrenNode *menu = jadren_node(menu_node);
    int index;
    if (menu == NULL || menu->kind != JADREN_NODE_MENU ||
        menu->menu_item_count >= JADREN_X11_MAX_MENU_ITEMS) {
        return 0;
    }
    index = menu->menu_item_count;
    menu->menu_item_events[index] = event_id;
    jadren_copy_text(menu->menu_item_text[index],
                     sizeof(menu->menu_item_text[index]), label_data, label_length);
    menu->menu_item_count += 1;
    return 1;
}

int32_t ui_app_select(int parent, int event_id, int width, int height,
                      uint32_t text_color, uint32_t background_color,
                      int corner_radius, int stretch) {
    int id;
    JadrenNode *node;
    (void)corner_radius;
    id = jadren_add_leaf(JADREN_NODE_SELECT, parent, "", 0, event_id,
                         width, height, text_color, background_color, stretch);
    node = jadren_node(id);
    if (node != NULL) {
        node->option_count = 0;
        node->selected_index = -1;
    }
    return id;
}

int32_t ui_app_select_option(int select_node, const char *text_data,
                             uint64_t text_length) {
    JadrenNode *select = jadren_node(select_node);
    int index;
    if (select == NULL || select->kind != JADREN_NODE_SELECT ||
        select->option_count >= JADREN_X11_MAX_MENU_ITEMS) {
        return 0;
    }
    index = select->option_count;
    jadren_copy_text(select->option_text[index],
                     sizeof(select->option_text[index]), text_data, text_length);
    select->option_count += 1;
    return 1;
}

int32_t ui_app_select_index(int select_node) {
    JadrenNode *select = jadren_node(select_node);
    return select == NULL || select->kind != JADREN_NODE_SELECT
               ? -1
               : select->selected_index;
}

int32_t ui_app_select_set_index(int select_node, int selected_index) {
    JadrenNode *select = jadren_node(select_node);
    if (select == NULL || select->kind != JADREN_NODE_SELECT ||
        selected_index < -1 || selected_index >= select->option_count) {
        return 0;
    }
    select->selected_index = selected_index;
    if (selected_index < 0) {
        select->text[0] = '\0';
    } else {
        jadren_copy_text(select->text, sizeof(select->text),
                         select->option_text[selected_index],
                         (uint64_t)strlen(select->option_text[selected_index]));
    }
    jadren_sync_select_app_state(select);
    return 1;
}

static JadrenNode *jadren_select_for_event(int event_id) {
    int index;
    for (index = 1; index < jadren_next_node; index += 1) {
        if (jadren_nodes[index].used &&
            jadren_nodes[index].kind == JADREN_NODE_SELECT &&
            jadren_nodes[index].event_id == event_id) {
            return &jadren_nodes[index];
        }
    }
    return NULL;
}

void ui_select_option(int event_id, const char *text_data, uint64_t text_length) {
    JadrenNode *select = jadren_select_for_event(event_id);
    if (select != NULL) {
        (void)ui_app_select_option(select->id, text_data, text_length);
    }
}

int32_t ui_select_index(int event_id) {
    JadrenNode *select = jadren_select_for_event(event_id);
    return select == NULL ? -1 : select->selected_index;
}

void ui_select_set_index(int event_id, int selected_index) {
    JadrenNode *select = jadren_select_for_event(event_id);
    if (select != NULL) {
        (void)ui_app_select_set_index(select->id, selected_index);
    }
}

int32_t ui_app_list(int parent, int event_id, int width, int height,
                    uint32_t text_color, uint32_t background_color,
                    int corner_radius, int stretch) {
    int id;
    JadrenNode *node;
    (void)corner_radius;
    id = jadren_add_leaf(JADREN_NODE_LIST, parent, "", 0, event_id,
                         width, height, text_color, background_color, stretch);
    node = jadren_node(id);
    if (node != NULL) {
        node->list_item_count = 0;
        node->list_selected_index = -1;
        node->app_list_id = -1;
    }
    return id;
}

int32_t ui_app_list_item(int list_node, const char *text_data,
                         uint64_t text_length) {
    JadrenNode *list = jadren_node(list_node);
    int index;
    if (list == NULL || list->kind != JADREN_NODE_LIST ||
        list->list_item_count >= JADREN_X11_MAX_LIST_ITEMS) {
        return 0;
    }
    index = list->list_item_count;
    jadren_copy_text(list->list_item_text[index],
                     sizeof(list->list_item_text[index]), text_data, text_length);
    list->list_item_count += 1;
    return 1;
}

int32_t ui_app_list_clear(int list_node) {
    JadrenNode *list = jadren_node(list_node);
    if (list == NULL || list->kind != JADREN_NODE_LIST) {
        return 0;
    }
    list->list_item_count = 0;
    list->list_selected_index = -1;
    jadren_sync_list_app_state(list);
    return 1;
}

int32_t ui_app_list_count(int list_node) {
    JadrenNode *list = jadren_node(list_node);
    return list == NULL || list->kind != JADREN_NODE_LIST
               ? 0
               : list->list_item_count;
}

int32_t ui_app_list_index(int list_node) {
    JadrenNode *list = jadren_node(list_node);
    return list == NULL || list->kind != JADREN_NODE_LIST
               ? -1
               : list->list_selected_index;
}

int32_t ui_app_list_set_index(int list_node, int selected_index) {
    JadrenNode *list = jadren_node(list_node);
    if (list == NULL || list->kind != JADREN_NODE_LIST ||
        selected_index < -1 || selected_index >= list->list_item_count) {
        return 0;
    }
    list->list_selected_index = selected_index;
    jadren_sync_list_app_state(list);
    return 1;
}

static int32_t jadren_refresh_list_from_app(JadrenNode *list) {
    unsigned char output[JADREN_X11_MAX_TEXT];
    int count;
    int selected_index;
    int index;
    if (list == NULL || list->kind != JADREN_NODE_LIST ||
        list->app_list_id < 0 || app_list_count == NULL ||
        app_list_read_text == NULL) {
        return 0;
    }
    selected_index = list->list_selected_index;
    count = app_list_count(list->app_list_id);
    if (count < 0) {
        count = 0;
    }
    if (count > JADREN_X11_MAX_LIST_ITEMS) {
        count = JADREN_X11_MAX_LIST_ITEMS;
    }
    list->list_item_count = count;
    list->list_selected_index = selected_index >= 0 && selected_index < count
                                    ? selected_index
                                    : -1;
    for (index = 0; index < count; index += 1) {
        uint64_t copied = app_list_read_text(
            list->app_list_id, index, output, sizeof(output));
        jadren_copy_text(list->list_item_text[index],
                         sizeof(list->list_item_text[index]),
                         (const char *)output, copied);
    }
    return 1;
}

int32_t ui_app_list_bind_app(int list_node, int list_id) {
    JadrenNode *list = jadren_node(list_node);
    if (list == NULL || list->kind != JADREN_NODE_LIST || list_id < 0) {
        return 0;
    }
    list->app_list_id = list_id;
    return jadren_refresh_list_from_app(list);
}

int32_t ui_app_list_refresh(int list_node) {
    JadrenNode *list = jadren_node(list_node);
    return jadren_refresh_list_from_app(list);
}

uint64_t ui_app_list_read_item(int list_node, int item_index,
                               unsigned char *output_data,
                               uint64_t output_length) {
    JadrenNode *list = jadren_node(list_node);
    size_t length;
    if (list == NULL || list->kind != JADREN_NODE_LIST || item_index < 0 ||
        item_index >= list->list_item_count || output_data == NULL) {
        return 0;
    }
    length = strlen(list->list_item_text[item_index]);
    if (output_length < (uint64_t)length) {
        return 0;
    }
    if (length > 0U) {
        memcpy(output_data, list->list_item_text[item_index], length);
    }
    return (uint64_t)length;
}

int32_t ui_app_table(int parent, int event_id, int width, int height,
                     uint32_t text_color, uint32_t background_color,
                     int corner_radius, int stretch) {
    int table_slot = jadren_free_table_slot();
    int id;
    JadrenNode *node;
    JadrenTableData *table;
    int column_index;
    (void)corner_radius;
    if (table_slot < 0) {
        return 0;
    }
    id = jadren_add_leaf(JADREN_NODE_TABLE, parent, "", 0, event_id,
                         width, height, text_color, background_color, stretch);
    node = jadren_node(id);
    if (node == NULL) {
        return 0;
    }
    table = &jadren_tables[table_slot];
    memset(table, 0, sizeof(*table));
    table->used = 1;
    table->selected_row = -1;
    table->app_table_id = -1;
    table->app_table_column_count = 0;
    for (column_index = 0; column_index < JADREN_X11_MAX_TABLE_COLUMNS;
         column_index += 1) {
        table->column_widths[column_index] = 160;
    }
    node->table_slot = table_slot;
    return id;
}

int32_t ui_app_table_column(int table_node, int column_index,
                            const char *title_data, uint64_t title_length,
                            int width) {
    JadrenTableData *table = jadren_table_data(jadren_node(table_node));
    int column_width;
    if (table == NULL || column_index < 0 ||
        column_index >= JADREN_X11_MAX_TABLE_COLUMNS || title_data == NULL ||
        title_length >= JADREN_X11_MAX_TEXT) {
        return 0;
    }
    column_width = width < 40 ? 40 : width > 800 ? 800 : width;
    jadren_copy_text(table->headers[column_index],
                     sizeof(table->headers[column_index]), title_data,
                     title_length);
    table->column_widths[column_index] = column_width;
    if (column_index >= table->column_count) {
        table->column_count = column_index + 1;
    }
    return 1;
}

int32_t ui_app_table_cell(int table_node, int row_index, int column_index,
                          const char *text_data, uint64_t text_length) {
    JadrenTableData *table = jadren_table_data(jadren_node(table_node));
    if (table == NULL || row_index < 0 ||
        row_index >= JADREN_X11_MAX_TABLE_ROWS || column_index < 0 ||
        column_index >= table->column_count || text_data == NULL ||
        text_length >= JADREN_X11_MAX_TEXT) {
        return 0;
    }
    jadren_copy_text(table->cells[row_index][column_index],
                     sizeof(table->cells[row_index][column_index]), text_data,
                     text_length);
    if (row_index >= table->row_count) {
        table->row_count = row_index + 1;
    }
    return 1;
}

uint64_t ui_app_table_read_cell(int table_node, int row_index,
                                int column_index, unsigned char *output_data,
                                uint64_t output_length) {
    JadrenTableData *table = jadren_table_data(jadren_node(table_node));
    size_t length;
    if (table == NULL || row_index < 0 || row_index >= table->row_count ||
        column_index < 0 || column_index >= table->column_count ||
        output_data == NULL) {
        return 0;
    }
    length = strlen(table->cells[row_index][column_index]);
    if (output_length < (uint64_t)length) {
        return 0;
    }
    if (length > 0U) {
        memcpy(output_data, table->cells[row_index][column_index], length);
    }
    return (uint64_t)length;
}

static int32_t jadren_refresh_table_from_app(JadrenNode *table_node) {
    unsigned char output[JADREN_X11_MAX_TEXT];
    JadrenTableData *table = jadren_table_data(table_node);
    int rows;
    int row_index;
    int column_index;
    if (table == NULL || table->app_table_id < 0 ||
        table->app_table_column_count < 1 || app_table_row_count == NULL ||
        app_table_read_cell == NULL) {
        return 0;
    }
    rows = app_table_row_count(table->app_table_id);
    if (rows < 0) {
        rows = 0;
    }
    if (rows > JADREN_X11_MAX_TABLE_ROWS) {
        rows = JADREN_X11_MAX_TABLE_ROWS;
    }
    table->column_count = table->app_table_column_count;
    table->row_count = rows;
    if (table->selected_row >= rows) {
        table->selected_row = -1;
    }
    for (row_index = 0; row_index < rows; row_index += 1) {
        for (column_index = 0; column_index < table->column_count;
             column_index += 1) {
            uint64_t copied = app_table_read_cell(
                table->app_table_id, row_index, column_index, output,
                sizeof(output));
            jadren_copy_text(table->cells[row_index][column_index],
                             sizeof(table->cells[row_index][column_index]),
                             (const char *)output, copied);
        }
    }
    return 1;
}

static void jadren_refresh_bound_tables(int app_table_id) {
    int node_index;
    for (node_index = 1; node_index < jadren_next_node; node_index += 1) {
        JadrenNode *node = &jadren_nodes[node_index];
        JadrenTableData *table;
        if (!node->used || node->kind != JADREN_NODE_TABLE) {
            continue;
        }
        table = jadren_table_data(node);
        if (table != NULL && table->app_table_id == app_table_id) {
            (void)jadren_refresh_table_from_app(node);
        }
    }
}

static int32_t jadren_sort_table_from_app(JadrenTableData *table,
                                          int column_index, int descending,
                                          int kind) {
    int (*sort_function)(int, int, int) = app_table_sort_text;
    if (kind == 1) sort_function = app_table_sort_int;
    else if (kind == 2) sort_function = app_table_sort_uint;
    else if (kind == 3) sort_function = app_table_sort_float;
    else if (kind == 4) sort_function = app_table_sort_bool;
    else if (kind != 0) return 0;
    if (table == NULL || table->app_table_id < 0 || sort_function == NULL ||
        column_index < 0 || column_index >= table->app_table_column_count) {
        return 0;
    }
    if (!sort_function(table->app_table_id, column_index, descending)) {
        return 0;
    }
    jadren_refresh_bound_tables(table->app_table_id);
    return 1;
}

int32_t ui_app_table_sort_text(int table_node, int column_index, int descending) {
    JadrenTableData *table = jadren_table_data(jadren_node(table_node));
    return jadren_sort_table_from_app(table, column_index, descending, 0);
}

int32_t ui_app_table_sort_int(int table_node, int column_index, int descending) {
    return jadren_sort_table_from_app(jadren_table_data(jadren_node(table_node)),
                                      column_index, descending, 1);
}

int32_t ui_app_table_sort_uint(int table_node, int column_index, int descending) {
    return jadren_sort_table_from_app(jadren_table_data(jadren_node(table_node)),
                                      column_index, descending, 2);
}

int32_t ui_app_table_sort_float(int table_node, int column_index, int descending) {
    return jadren_sort_table_from_app(jadren_table_data(jadren_node(table_node)),
                                      column_index, descending, 3);
}

int32_t ui_app_table_sort_bool(int table_node, int column_index, int descending) {
    return jadren_sort_table_from_app(jadren_table_data(jadren_node(table_node)),
                                      column_index, descending, 4);
}

void ui_table_sort_text(int event_id, int column_index, int descending) {
    (void)jadren_sort_table_from_app(
        jadren_table_data(jadren_node_for_event(event_id, JADREN_NODE_TABLE)),
        column_index, descending, 0);
}

void ui_table_sort_int(int event_id, int column_index, int descending) {
    (void)jadren_sort_table_from_app(
        jadren_table_data(jadren_node_for_event(event_id, JADREN_NODE_TABLE)),
        column_index, descending, 1);
}

void ui_table_sort_uint(int event_id, int column_index, int descending) {
    (void)jadren_sort_table_from_app(
        jadren_table_data(jadren_node_for_event(event_id, JADREN_NODE_TABLE)),
        column_index, descending, 2);
}

void ui_table_sort_float(int event_id, int column_index, int descending) {
    (void)jadren_sort_table_from_app(
        jadren_table_data(jadren_node_for_event(event_id, JADREN_NODE_TABLE)),
        column_index, descending, 3);
}

void ui_table_sort_bool(int event_id, int column_index, int descending) {
    (void)jadren_sort_table_from_app(
        jadren_table_data(jadren_node_for_event(event_id, JADREN_NODE_TABLE)),
        column_index, descending, 4);
}

int32_t ui_app_table_filter_text(int table_node, int destination_table_id,
                                 int column_index, const char *query_data,
                                 uint64_t query_length) {
    JadrenTableData *table = jadren_table_data(jadren_node(table_node));
    if (table == NULL || table->app_table_id < 0 || destination_table_id < 0 ||
        destination_table_id == table->app_table_id || app_table_filter_text_ex == NULL ||
        column_index < 0 || column_index >= table->app_table_column_count ||
        (query_data == NULL && query_length > 0U)) {
        return 0;
    }
    if (!app_table_filter_text_ex(table->app_table_id, destination_table_id,
                                  column_index, query_data, query_length, 0)) {
        return 0;
    }
    jadren_refresh_bound_tables(destination_table_id);
    return 1;
}

int32_t ui_app_table_filter_text_ex(int table_node, int destination_table_id,
                                    int column_index, const char *query_data,
                                    uint64_t query_length, int mode) {
    JadrenTableData *table = jadren_table_data(jadren_node(table_node));
    if (table == NULL || table->app_table_id < 0 || destination_table_id < 0 ||
        destination_table_id == table->app_table_id || app_table_filter_text_ex == NULL ||
        column_index < 0 || column_index >= table->app_table_column_count ||
        (query_data == NULL && query_length > 0U) || mode < 0 || mode > 7) {
        return 0;
    }
    if (!app_table_filter_text_ex(table->app_table_id, destination_table_id,
                                  column_index, query_data, query_length, mode)) {
        return 0;
    }
    jadren_refresh_bound_tables(destination_table_id);
    return 1;
}

static int32_t jadren_filter_table_typed(JadrenTableData *table,
                                          int destination_table_id,
                                          int column_index, int kind,
                                          int64_t int_query,
                                          uint64_t uint_query,
                                          double float_query,
                                          unsigned char bool_query) {
    int result = 0;
    if (table == NULL || table->app_table_id < 0 || destination_table_id < 0 ||
        destination_table_id == table->app_table_id || column_index < 0 ||
        column_index >= table->app_table_column_count) return 0;
    if (kind == 1 && app_table_filter_int != NULL)
        result = app_table_filter_int(table->app_table_id, destination_table_id,
                                      column_index, int_query);
    else if (kind == 2 && app_table_filter_uint != NULL)
        result = app_table_filter_uint(table->app_table_id, destination_table_id,
                                       column_index, uint_query);
    else if (kind == 3 && app_table_filter_float != NULL)
        result = app_table_filter_float(table->app_table_id, destination_table_id,
                                        column_index, float_query);
    else if (kind == 4 && app_table_filter_bool != NULL)
        result = app_table_filter_bool(table->app_table_id, destination_table_id,
                                       column_index, bool_query);
    if (!result) return 0;
    jadren_refresh_bound_tables(destination_table_id);
    return 1;
}

int32_t ui_app_table_filter_int(int table_node, int destination_table_id,
                                int column_index, int64_t query) {
    return jadren_filter_table_typed(jadren_table_data(jadren_node(table_node)),
                                     destination_table_id, column_index, 1,
                                     query, 0, 0.0, 0);
}

int32_t ui_app_table_filter_uint(int table_node, int destination_table_id,
                                 int column_index, uint64_t query) {
    return jadren_filter_table_typed(jadren_table_data(jadren_node(table_node)),
                                     destination_table_id, column_index, 2,
                                     0, query, 0.0, 0);
}

int32_t ui_app_table_filter_float(int table_node, int destination_table_id,
                                  int column_index, double query) {
    return jadren_filter_table_typed(jadren_table_data(jadren_node(table_node)),
                                     destination_table_id, column_index, 3,
                                     0, 0, query, 0);
}

int32_t ui_app_table_filter_bool(int table_node, int destination_table_id,
                                 int column_index, unsigned char query) {
    return jadren_filter_table_typed(jadren_table_data(jadren_node(table_node)),
                                     destination_table_id, column_index, 4,
                                     0, 0, 0.0, query);
}

void ui_table_filter_int(int event_id, int destination_table_id, int column_index,
                         int64_t query) {
    (void)jadren_filter_table_typed(
        jadren_table_data(jadren_node_for_event(event_id, JADREN_NODE_TABLE)),
        destination_table_id, column_index, 1, query, 0, 0.0, 0);
}

void ui_table_filter_uint(int event_id, int destination_table_id, int column_index,
                          uint64_t query) {
    (void)jadren_filter_table_typed(
        jadren_table_data(jadren_node_for_event(event_id, JADREN_NODE_TABLE)),
        destination_table_id, column_index, 2, 0, query, 0.0, 0);
}

void ui_table_filter_float(int event_id, int destination_table_id, int column_index,
                           double query) {
    (void)jadren_filter_table_typed(
        jadren_table_data(jadren_node_for_event(event_id, JADREN_NODE_TABLE)),
        destination_table_id, column_index, 3, 0, 0, query, 0);
}

void ui_table_filter_bool(int event_id, int destination_table_id, int column_index,
                          unsigned char query) {
    (void)jadren_filter_table_typed(
        jadren_table_data(jadren_node_for_event(event_id, JADREN_NODE_TABLE)),
        destination_table_id, column_index, 4, 0, 0, 0.0, query);
}

int32_t ui_app_table_bind_app(int table_node, int table_id,
                              int column_count) {
    JadrenTableData *table = jadren_table_data(jadren_node(table_node));
    int column_index;
    if (table == NULL || table_id < 0 || column_count < 1 ||
        column_count > JADREN_X11_MAX_TABLE_COLUMNS) {
        return 0;
    }
    table->app_table_id = table_id;
    table->app_table_column_count = column_count;
    for (column_index = 0; column_index < column_count; column_index += 1) {
        if (table->column_widths[column_index] < 40) {
            table->column_widths[column_index] = 160;
        }
    }
    return jadren_refresh_table_from_app(jadren_node(table_node));
}

int32_t ui_app_table_refresh(int table_node) {
    return jadren_refresh_table_from_app(jadren_node(table_node));
}

int32_t ui_app_table_clear(int table_node) {
    JadrenTableData *table = jadren_table_data(jadren_node(table_node));
    if (table == NULL) {
        return 0;
    }
    table->row_count = 0;
    table->selected_row = -1;
    jadren_sync_table_app_state(jadren_node(table_node));
    return 1;
}

int32_t ui_app_table_row_count(int table_node) {
    JadrenTableData *table = jadren_table_data(jadren_node(table_node));
    return table == NULL ? 0 : table->row_count;
}

int32_t ui_app_table_selected_row(int table_node) {
    JadrenTableData *table = jadren_table_data(jadren_node(table_node));
    return table == NULL ? -1 : table->selected_row;
}

int32_t ui_app_table_set_selected_row(int table_node, int row_index) {
    JadrenTableData *table = jadren_table_data(jadren_node(table_node));
    if (table == NULL || row_index < -1 || row_index >= table->row_count) {
        return 0;
    }
    table->selected_row = row_index;
    jadren_sync_table_app_state(jadren_node(table_node));
    return 1;
}

enum {
    JADREN_STATE_BIND_CHECKED = 0,
    JADREN_STATE_BIND_INDEX = 1,
    JADREN_STATE_BIND_INPUT_LENGTH = 2,
    JADREN_STATE_BIND_COUNT = 3,
    JADREN_STATE_BIND_TEXT = 4
};

static JadrenNode *jadren_node_for_event(int event_id, int kind) {
    int index;
    for (index = 1; index < jadren_next_node; index += 1) {
        if (jadren_nodes[index].used &&
            jadren_nodes[index].event_id == event_id &&
            (kind == 0 || jadren_nodes[index].kind == kind)) {
            return &jadren_nodes[index];
        }
    }
    return NULL;
}

static int jadren_state_binding_value(const JadrenStateBinding *binding) {
    JadrenNode *node;
    JadrenTableData *table;
    if (binding == NULL) {
        return 0;
    }
    switch (binding->mode) {
    case JADREN_STATE_BIND_CHECKED:
        node = jadren_node_for_event(binding->event_id, JADREN_NODE_CHECKBOX);
        return node == NULL ? 0 : node->checked != 0;
    case JADREN_STATE_BIND_INDEX:
        node = jadren_node_for_event(binding->event_id, JADREN_NODE_SELECT);
        if (node != NULL) {
            return node->selected_index;
        }
        node = jadren_node_for_event(binding->event_id, JADREN_NODE_LIST);
        if (node != NULL) {
            return node->list_selected_index;
        }
        node = jadren_node_for_event(binding->event_id, JADREN_NODE_TABLE);
        table = jadren_table_data(node);
        return table == NULL ? -1 : table->selected_row;
    case JADREN_STATE_BIND_INPUT_LENGTH:
        node = jadren_node_for_event(binding->event_id, JADREN_NODE_INPUT);
        return node == NULL ? 0 : (int)strlen(node->text);
    case JADREN_STATE_BIND_COUNT:
        node = jadren_node_for_event(binding->event_id, JADREN_NODE_LIST);
        if (node != NULL) {
            return node->list_item_count;
        }
        node = jadren_node_for_event(binding->event_id, JADREN_NODE_TABLE);
        table = jadren_table_data(node);
        return table == NULL ? 0 : table->row_count;
    default:
        return 0;
    }
}

static void jadren_apply_state_binding(const JadrenStateBinding *binding) {
    JadrenNode *node;
    JadrenTableData *table;
    int value;
    if (binding == NULL || binding->slot < 0 ||
        binding->slot >= JADREN_X11_MAX_STATE_SLOTS) {
        return;
    }
    value = jadren_state_slots[binding->slot];
    switch (binding->mode) {
    case JADREN_STATE_BIND_CHECKED:
        node = jadren_node_for_event(binding->event_id, JADREN_NODE_CHECKBOX);
        if (node != NULL) {
            node->checked = value != 0;
        }
        break;
    case JADREN_STATE_BIND_INDEX:
        node = jadren_node_for_event(binding->event_id, JADREN_NODE_SELECT);
        if (node != NULL && value >= -1 && value < node->option_count) {
            node->selected_index = value;
            if (value < 0) {
                node->text[0] = '\0';
            } else {
                jadren_copy_text(node->text, sizeof(node->text),
                                 node->option_text[value],
                                 (uint64_t)strlen(node->option_text[value]));
            }
            break;
        }
        node = jadren_node_for_event(binding->event_id, JADREN_NODE_LIST);
        if (node != NULL && value >= -1 && value < node->list_item_count) {
            node->list_selected_index = value;
            break;
        }
        node = jadren_node_for_event(binding->event_id, JADREN_NODE_TABLE);
        table = jadren_table_data(node);
        if (table != NULL && value >= -1 && value < table->row_count) {
            table->selected_row = value;
        }
        break;
    case JADREN_STATE_BIND_INPUT_LENGTH:
    case JADREN_STATE_BIND_COUNT:
    case JADREN_STATE_BIND_TEXT:
    default:
        break;
    }
}

static JadrenStateBinding *jadren_find_state_binding(int event_id, int mode) {
    int index;
    for (index = 0; index < jadren_state_binding_count; index += 1) {
        if (jadren_state_bindings[index].event_id == event_id &&
            jadren_state_bindings[index].mode == mode) {
            return &jadren_state_bindings[index];
        }
    }
    return NULL;
}

/* Programmatic event dispatch shares the same callback and state-sync path as
 * a native X11 click/change event. Unknown ids are rejected deterministically. */
int32_t ui_dispatch_event(int32_t event_id) {
#if JADREN_UI_HAS_EVENT_CALLBACK
    int index;
    if (event_id == 0) {
        return 0;
    }
    for (index = 1; index < jadren_next_node; index += 1) {
        JadrenNode *node = &jadren_nodes[index];
        if (!node->used || node->event_id != event_id) {
            continue;
        }
        if (node->kind == JADREN_NODE_BUTTON || node->kind == JADREN_NODE_CHECKBOX ||
            node->kind == JADREN_NODE_INPUT || node->kind == JADREN_NODE_SELECT ||
            node->kind == JADREN_NODE_LIST || node->kind == JADREN_NODE_TABLE) {
            jadren_dispatch_event(event_id);
            return 1;
        }
    }
#else
    (void)event_id;
#endif
    return 0;
}

static void jadren_sync_state_bindings_for_event(int event_id) {
    int index;
    for (index = 0; index < jadren_state_binding_count; index += 1) {
        JadrenStateBinding *binding = &jadren_state_bindings[index];
        if (binding->event_id != event_id || binding->slot < 0 ||
            binding->slot >= JADREN_X11_MAX_STATE_SLOTS) {
            continue;
        }
        if (binding->mode == JADREN_STATE_BIND_TEXT) {
            JadrenNode *input = jadren_node_for_event(event_id, JADREN_NODE_INPUT);
            if (input != NULL) {
                jadren_copy_text(jadren_state_text[binding->slot],
                                 sizeof(jadren_state_text[binding->slot]),
                                 input->text, (uint64_t)strlen(input->text));
            }
        } else {
            jadren_state_slots[binding->slot] =
                jadren_state_binding_value(binding);
        }
    }
}

void ui_state_bind(int event_id, int slot, int mode) {
    JadrenStateBinding *binding;
    if (slot < 0 || slot >= JADREN_X11_MAX_STATE_SLOTS ||
        mode < JADREN_STATE_BIND_CHECKED || mode > JADREN_STATE_BIND_COUNT) {
        return;
    }
    binding = jadren_find_state_binding(event_id, mode);
    if (binding == NULL) {
        if (jadren_state_binding_count >= JADREN_X11_MAX_STATE_BINDINGS) {
            return;
        }
        binding = &jadren_state_bindings[jadren_state_binding_count];
        jadren_state_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->slot = slot;
    binding->mode = mode;
    if (mode == JADREN_STATE_BIND_INPUT_LENGTH ||
        mode == JADREN_STATE_BIND_COUNT) {
        jadren_state_slots[slot] = jadren_state_binding_value(binding);
    } else {
        jadren_apply_state_binding(binding);
    }
}

void ui_state_bind_text(int event_id, int slot) {
    JadrenStateBinding *binding;
    JadrenNode *input;
    if (slot < 0 || slot >= JADREN_X11_MAX_STATE_SLOTS) {
        return;
    }
    binding = jadren_find_state_binding(event_id, JADREN_STATE_BIND_TEXT);
    if (binding == NULL) {
        if (jadren_state_binding_count >= JADREN_X11_MAX_STATE_BINDINGS) {
            return;
        }
        binding = &jadren_state_bindings[jadren_state_binding_count];
        jadren_state_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->slot = slot;
    binding->mode = JADREN_STATE_BIND_TEXT;
    input = jadren_node_for_event(event_id, JADREN_NODE_INPUT);
    if (input == NULL) {
        jadren_state_text[slot][0] = '\0';
    } else {
        jadren_copy_text(jadren_state_text[slot], sizeof(jadren_state_text[slot]),
                         input->text, (uint64_t)strlen(input->text));
    }
}

int32_t ui_state_get(int slot) {
    return slot < 0 || slot >= JADREN_X11_MAX_STATE_SLOTS
               ? 0
               : jadren_state_slots[slot];
}

void ui_state_set(int slot, int value) {
    int index;
    if (slot < 0 || slot >= JADREN_X11_MAX_STATE_SLOTS) {
        return;
    }
    jadren_state_slots[slot] = value;
    for (index = 0; index < jadren_state_binding_count; index += 1) {
        if (jadren_state_bindings[index].slot == slot) {
            jadren_apply_state_binding(&jadren_state_bindings[index]);
        }
    }
}

uint64_t ui_state_text_length(int slot) {
    return slot < 0 || slot >= JADREN_X11_MAX_STATE_SLOTS
               ? 0U
               : (uint64_t)strlen(jadren_state_text[slot]);
}

uint64_t ui_state_text_read(int slot, unsigned char *output_data,
                            uint64_t output_length) {
    uint64_t length;
    if (slot < 0 || slot >= JADREN_X11_MAX_STATE_SLOTS ||
        output_data == NULL) {
        return 0U;
    }
    length = (uint64_t)strlen(jadren_state_text[slot]);
    if (output_length < length) {
        return 0U;
    }
    if (length > 0U) {
        memcpy(output_data, jadren_state_text[slot], (size_t)length);
    }
    return length;
}

int32_t ui_state_text_set(int slot, const char *text_data,
                          uint64_t text_length) {
    int index;
    if (slot < 0 || slot >= JADREN_X11_MAX_STATE_SLOTS ||
        (text_data == NULL && text_length > 0U)) {
        return 0;
    }
    jadren_copy_text(jadren_state_text[slot], sizeof(jadren_state_text[slot]),
                     text_data, text_length);
    for (index = 0; index < jadren_state_binding_count; index += 1) {
        JadrenStateBinding *binding = &jadren_state_bindings[index];
        JadrenNode *input;
        if (binding->slot != slot || binding->mode != JADREN_STATE_BIND_TEXT) {
            continue;
        }
        input = jadren_node_for_event(binding->event_id, JADREN_NODE_INPUT);
        if (input != NULL) {
            jadren_copy_text(input->text, sizeof(input->text),
                             jadren_state_text[slot],
                             (uint64_t)strlen(jadren_state_text[slot]));
            jadren_sync_input_app_state(input);
        }
    }
    return 1;
}

/* Refresh every retained projection at one explicit application boundary.
 * The source model remains app_state/app_list/app_table; X11 only re-reads
 * bounded projections and never invents a second reactive graph. */
void ui_refresh_bindings(void) {
    int index;
    for (index = 1; index < jadren_next_node; index += 1) {
        JadrenNode *node = &jadren_nodes[index];
        if (!node->used) {
            continue;
        }
        JadrenInputAppBinding *app_binding =
            jadren_input_app_binding_for_event(node->event_id);
        if (node->kind == JADREN_NODE_INPUT && app_binding != NULL &&
            app_binding->kind == JADREN_APP_BIND_TEXT) {
            jadren_refresh_input_from_app_state(node);
        } else if (node->kind == JADREN_NODE_CHECKBOX && app_binding != NULL &&
                   app_binding->kind == JADREN_APP_BIND_BOOL) {
            jadren_refresh_checkbox_from_app_state(node);
        } else if (node->kind == JADREN_NODE_SELECT && app_binding != NULL &&
                   app_binding->kind == JADREN_APP_BIND_SELECT) {
            jadren_refresh_select_from_app_state(node);
        } else if (node->kind == JADREN_NODE_LIST) {
            if (node->app_list_id >= 0) {
                (void)jadren_refresh_list_from_app(node);
            }
            jadren_refresh_list_from_app_state(node);
        } else if (node->kind == JADREN_NODE_TABLE) {
            JadrenTableData *table = jadren_table_data(node);
            if (table != NULL && table->app_table_id >= 0) {
                (void)jadren_refresh_table_from_app(node);
            }
            jadren_refresh_table_from_app_state(node);
        }
    }
    for (index = 0; index < jadren_state_binding_count; index += 1) {
        jadren_sync_state_bindings_for_event(
            jadren_state_bindings[index].event_id);
    }
}

int32_t ui_checked(int event_id) {
    JadrenNode *checkbox =
        jadren_node_for_event(event_id, JADREN_NODE_CHECKBOX);
    return checkbox == NULL ? 0 : checkbox->checked != 0;
}

void ui_set_checked(int event_id, int checked) {
    JadrenNode *checkbox =
        jadren_node_for_event(event_id, JADREN_NODE_CHECKBOX);
    if (checkbox != NULL) {
        checkbox->checked = checked != 0;
        jadren_sync_checkbox_app_state(checkbox);
        jadren_sync_state_bindings_for_event(event_id);
    }
}

int32_t ui_app_label(int parent, const char *text_data, uint64_t text_length,
                     int width, int height, uint32_t text_color,
                     uint32_t background_color, int corner_radius, int stretch) {
    (void)corner_radius;
    return jadren_add_leaf(JADREN_NODE_LABEL, parent, text_data, text_length,
                           0, width, height, text_color, background_color, stretch);
}

int32_t ui_app_status(int parent, const char *text_data, uint64_t text_length,
                      int width, int height, uint32_t text_color,
                      uint32_t background_color, int corner_radius, int stretch) {
    int id;
    (void)corner_radius;
    id = jadren_add_leaf(JADREN_NODE_STATUS, parent, text_data, text_length,
                         0, width, height, text_color, background_color, stretch);
    return id;
}

int32_t ui_app_text_input(int parent, const char *text_data,
                          uint64_t text_length, int event_id, int width,
                          int height, uint32_t text_color,
                          uint32_t background_color, int corner_radius,
                          int stretch) {
    (void)corner_radius;
    return jadren_add_leaf(JADREN_NODE_INPUT, parent, text_data, text_length,
                           event_id, width, height, text_color,
                           background_color, stretch);
}

int32_t ui_app_button(int parent, const char *label_data, uint64_t label_length,
                      int event_id, int width, int height, uint32_t text_color,
                      uint32_t background_color, int corner_radius, int stretch) {
    (void)corner_radius;
    return jadren_add_leaf(JADREN_NODE_BUTTON, parent, label_data, label_length,
                           event_id, width, height, text_color, background_color, stretch);
}

int32_t ui_app_tooltip(int node_id, const char *text_data, uint64_t text_length,
                       int width, int height, uint32_t text_color,
                       uint32_t background_color, int corner_radius) {
    JadrenNode *node = jadren_node(node_id);
    if (node == NULL || node->event_id == 0 ||
        (node->kind != JADREN_NODE_BUTTON && node->kind != JADREN_NODE_CHECKBOX) ||
        text_data == NULL) {
        return 0;
    }
    node->tooltip_defined = 1;
    node->tooltip_width = width;
    node->tooltip_height = height;
    node->tooltip_text_color = text_color;
    node->tooltip_background_color = background_color;
    node->tooltip_corner_radius = corner_radius;
    jadren_copy_text(node->tooltip_text, sizeof(node->tooltip_text),
                     text_data, text_length);
    return 1;
}

int32_t ui_app_checkbox(int parent, const char *label_data, uint64_t label_length,
                        int event_id, int width, int height, uint32_t text_color,
                        uint32_t background_color, int corner_radius, int stretch,
                        int checked) {
    int id;
    (void)corner_radius;
    id = jadren_add_leaf(JADREN_NODE_CHECKBOX, parent, label_data, label_length,
                         event_id, width, height, text_color, background_color, stretch);
    if (id != 0) {
        jadren_nodes[id].checked = checked != 0;
    }
    return id;
}

int32_t ui_app_end(int node) {
    if (!jadren_active || jadren_stack_depth <= 0 ||
        jadren_stack[jadren_stack_depth - 1] != node) {
        return 0;
    }
    jadren_stack_depth -= 1;
    if (jadren_stack_depth == 0) {
        jadren_active = 0;
    }
    return 1;
}

void ui_set_status(const char *text_data, uint64_t text_length) {
    JadrenNode *node = jadren_node(jadren_status_node);
    if (node != NULL) {
        jadren_copy_text(node->text, sizeof(node->text), text_data, text_length);
    }
}

void ui_set_button_text(int event_id, const char *text_data, uint64_t text_length) {
    int index;
    for (index = 1; index < jadren_next_node; index += 1) {
        if (jadren_nodes[index].used && jadren_nodes[index].event_id == event_id &&
            (jadren_nodes[index].kind == JADREN_NODE_BUTTON ||
             jadren_nodes[index].kind == JADREN_NODE_CHECKBOX)) {
            jadren_copy_text(jadren_nodes[index].text,
                             sizeof(jadren_nodes[index].text), text_data, text_length);
            return;
        }
    }
}

static JadrenNode *jadren_input_for_event(int event_id) {
    int index;
    for (index = 1; index < jadren_next_node; index += 1) {
        if (jadren_nodes[index].used &&
            jadren_nodes[index].kind == JADREN_NODE_INPUT &&
            jadren_nodes[index].event_id == event_id) {
            return &jadren_nodes[index];
        }
    }
    return NULL;
}

void ui_set_input_text(int event_id, const char *text_data, uint64_t text_length) {
    JadrenNode *node = jadren_input_for_event(event_id);
    if (node != NULL) {
        jadren_copy_text(node->text, sizeof(node->text), text_data, text_length);
        jadren_sync_input_app_state(node);
        jadren_sync_state_bindings_for_event(event_id);
    }
}

uint64_t ui_input_length(int event_id) {
    JadrenNode *node = jadren_input_for_event(event_id);
    return node == NULL ? 0U : (uint64_t)strlen(node->text);
}

uint64_t ui_input_read(int event_id, unsigned char *output_data,
                       uint64_t output_length) {
    JadrenNode *node = jadren_input_for_event(event_id);
    uint64_t length;
    if (node == NULL || output_data == NULL || output_length == 0U) {
        return 0U;
    }
    length = (uint64_t)strlen(node->text);
    if (length > output_length) {
        length = output_length;
    }
    if (length > 0U) {
        memcpy(output_data, node->text, (size_t)length);
    }
    return length;
}

/* Exact read-back variant. A valid empty input returns true with length zero;
 * partial writes are rejected and caller-owned outputs stay unchanged. */
int ui_input_read_exact(int event_id, unsigned char *output_data,
                        uint64_t output_length,
                        uint64_t *output_text_length,
                        uint64_t output_text_length_capacity) {
    JadrenNode *node = jadren_input_for_event(event_id);
    uint64_t length;
    if (node == NULL || output_text_length == NULL ||
        output_text_length_capacity == 0U) {
        return 0;
    }
    length = (uint64_t)strlen(node->text);
    if (length > output_length || (length > 0U && output_data == NULL)) {
        return 0;
    }
    if (length > 0U) {
        memcpy(output_data, node->text, (size_t)length);
    }
    output_text_length[0] = length;
    return 1;
}

static JadrenInputAppBinding *jadren_input_app_binding_for_event(int event_id) {
    int index;
    for (index = 0; index < jadren_input_app_binding_count; index += 1) {
        if (jadren_input_app_bindings[index].event_id == event_id) {
            return &jadren_input_app_bindings[index];
        }
    }
    return NULL;
}

static void jadren_sync_input_app_state(JadrenNode *input) {
#if JADREN_UI_HAS_FILE_RUNTIME
    JadrenInputAppBinding *binding;
    if (input == NULL) {
        return;
    }
    binding = jadren_input_app_binding_for_event(input->event_id);
    if (binding == NULL || binding->kind != JADREN_APP_BIND_TEXT) {
        return;
    }
    (void)app_state_set_text(binding->key, (uint64_t)binding->key_length,
                             input->text, (uint64_t)strlen(input->text));
#else
    (void)input;
#endif
}

static void jadren_refresh_input_from_app_state(JadrenNode *input) {
#if JADREN_UI_HAS_FILE_RUNTIME
    JadrenInputAppBinding *binding;
    unsigned char output[JADREN_X11_MAX_TEXT];
    uint64_t copied;
    if (input == NULL) {
        return;
    }
    binding = jadren_input_app_binding_for_event(input->event_id);
    if (binding == NULL || binding->kind != JADREN_APP_BIND_TEXT) {
        return;
    }
    copied = app_state_read_text(binding->key, (uint64_t)binding->key_length,
                                 output, sizeof(output));
    jadren_copy_text(input->text, sizeof(input->text), (const char *)output,
                     copied);
    jadren_sync_state_bindings_for_event(input->event_id);
#else
    (void)input;
#endif
}

void ui_input_bind_app_state(int event_id, const char *key_data,
                             uint64_t key_length) {
    JadrenInputAppBinding *binding;
    JadrenNode *input = jadren_input_for_event(event_id);
    if (input == NULL || key_data == NULL || key_length == 0U ||
        key_length > 64U) {
        return;
    }
    binding = jadren_input_app_binding_for_event(event_id);
    if (binding == NULL) {
        if (jadren_input_app_binding_count >= JADREN_X11_MAX_INPUT_APP_BINDINGS) {
            return;
        }
        binding = &jadren_input_app_bindings[jadren_input_app_binding_count];
        jadren_input_app_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->kind = JADREN_APP_BIND_TEXT;
    binding->key_length = (size_t)key_length;
    jadren_copy_text(binding->key, sizeof(binding->key), key_data, key_length);
    if (!jadren_preserve_app_binding) {
        jadren_sync_input_app_state(input);
    }
}

void ui_input_refresh_app_state(int event_id) {
    jadren_refresh_input_from_app_state(jadren_input_for_event(event_id));
}

static void jadren_sync_checkbox_app_state(JadrenNode *checkbox) {
#if JADREN_UI_HAS_FILE_RUNTIME
    JadrenInputAppBinding *binding;
    if (checkbox == NULL) return;
    binding = jadren_input_app_binding_for_event(checkbox->event_id);
    if (binding == NULL || binding->kind != JADREN_APP_BIND_BOOL) return;
    (void)app_state_set_bool(binding->key, (uint64_t)binding->key_length,
                             (unsigned char)(checkbox->checked ? 1 : 0));
#else
    (void)checkbox;
#endif
}

static void jadren_refresh_checkbox_from_app_state(JadrenNode *checkbox) {
#if JADREN_UI_HAS_FILE_RUNTIME
    JadrenInputAppBinding *binding;
    if (checkbox == NULL) return;
    binding = jadren_input_app_binding_for_event(checkbox->event_id);
    if (binding == NULL || binding->kind != JADREN_APP_BIND_BOOL) return;
    checkbox->checked = app_state_get_bool(binding->key,
                                           (uint64_t)binding->key_length) != 0;
    jadren_sync_state_bindings_for_event(checkbox->event_id);
#else
    (void)checkbox;
#endif
}

static void jadren_sync_select_app_state(JadrenNode *select) {
#if JADREN_UI_HAS_FILE_RUNTIME
    JadrenInputAppBinding *binding;
    if (select == NULL) return;
    binding = jadren_input_app_binding_for_event(select->event_id);
    if (binding == NULL || binding->kind != JADREN_APP_BIND_SELECT) return;
    (void)app_state_set_int(binding->key, (uint64_t)binding->key_length,
                            (int64_t)select->selected_index);
#else
    (void)select;
#endif
}

static void jadren_refresh_select_from_app_state(JadrenNode *select) {
#if JADREN_UI_HAS_FILE_RUNTIME
    JadrenInputAppBinding *binding;
    int64_t value;
    if (select == NULL) return;
    binding = jadren_input_app_binding_for_event(select->event_id);
    if (binding == NULL || binding->kind != JADREN_APP_BIND_SELECT) return;
    value = app_state_get_int(binding->key, (uint64_t)binding->key_length);
    if (value < -1 || value >= (int64_t)select->option_count) value = -1;
    select->selected_index = (int)value;
    if (value < 0) {
        select->text[0] = '\0';
    } else {
        jadren_copy_text(select->text, sizeof(select->text),
                         select->option_text[value],
                         (uint64_t)strlen(select->option_text[value]));
    }
    jadren_sync_state_bindings_for_event(select->event_id);
#else
    (void)select;
#endif
}

int ui_checkbox_bind_app_state(int event_id, const char *key_data,
                               uint64_t key_length) {
    JadrenInputAppBinding *binding;
    JadrenNode *checkbox = jadren_node_for_event(event_id, JADREN_NODE_CHECKBOX);
    if (checkbox == NULL || key_data == NULL || key_length == 0U ||
        key_length > 64U) {
        return 0;
    }
    binding = jadren_input_app_binding_for_event(event_id);
    if (binding == NULL) {
        if (jadren_input_app_binding_count >= JADREN_X11_MAX_INPUT_APP_BINDINGS) {
            return 0;
        }
        binding = &jadren_input_app_bindings[jadren_input_app_binding_count];
        jadren_input_app_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->kind = JADREN_APP_BIND_BOOL;
    binding->key_length = (size_t)key_length;
    jadren_copy_text(binding->key, sizeof(binding->key), key_data, key_length);
#if JADREN_UI_HAS_FILE_RUNTIME
    if (!jadren_preserve_app_binding &&
        !app_state_set_bool(binding->key, (uint64_t)binding->key_length,
                            (unsigned char)(checkbox->checked ? 1 : 0))) {
        return 0;
    }
#endif
    if (!jadren_preserve_app_binding) {
        jadren_sync_checkbox_app_state(checkbox);
    }
    return 1;
}

void ui_checkbox_refresh_app_state(int event_id) {
    jadren_refresh_checkbox_from_app_state(
        jadren_node_for_event(event_id, JADREN_NODE_CHECKBOX));
}

int ui_select_bind_app_state(int event_id, const char *key_data,
                             uint64_t key_length) {
    JadrenInputAppBinding *binding;
    JadrenNode *select = jadren_node_for_event(event_id, JADREN_NODE_SELECT);
    if (select == NULL || key_data == NULL || key_length == 0U ||
        key_length > 64U) {
        return 0;
    }
    binding = jadren_input_app_binding_for_event(event_id);
    if (binding == NULL) {
        if (jadren_input_app_binding_count >= JADREN_X11_MAX_INPUT_APP_BINDINGS) {
            return 0;
        }
        binding = &jadren_input_app_bindings[jadren_input_app_binding_count];
        jadren_input_app_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->kind = JADREN_APP_BIND_SELECT;
    binding->key_length = (size_t)key_length;
    jadren_copy_text(binding->key, sizeof(binding->key), key_data, key_length);
#if JADREN_UI_HAS_FILE_RUNTIME
    if (!jadren_preserve_app_binding &&
        !app_state_set_int(binding->key, (uint64_t)binding->key_length,
                           (int64_t)select->selected_index)) {
        return 0;
    }
#endif
    if (!jadren_preserve_app_binding) {
        jadren_sync_select_app_state(select);
    }
    return 1;
}

void ui_select_refresh_app_state(int event_id) {
    jadren_refresh_select_from_app_state(
        jadren_node_for_event(event_id, JADREN_NODE_SELECT));
}

static void jadren_sync_list_app_state(JadrenNode *list) {
#if JADREN_UI_HAS_FILE_RUNTIME
    JadrenInputAppBinding *binding;
    if (list == NULL) return;
    binding = jadren_input_app_binding_for_event(list->event_id);
    if (binding == NULL || binding->kind != JADREN_APP_BIND_LIST) return;
    (void)app_state_set_int(binding->key, (uint64_t)binding->key_length,
                            (int64_t)list->list_selected_index);
#else
    (void)list;
#endif
}

static void jadren_refresh_list_from_app_state(JadrenNode *list) {
#if JADREN_UI_HAS_FILE_RUNTIME
    JadrenInputAppBinding *binding;
    int64_t value;
    if (list == NULL) return;
    binding = jadren_input_app_binding_for_event(list->event_id);
    if (binding == NULL || binding->kind != JADREN_APP_BIND_LIST) return;
    value = app_state_get_int(binding->key, (uint64_t)binding->key_length);
    if (value < -1 || value >= (int64_t)list->list_item_count) value = -1;
    list->list_selected_index = (int)value;
    jadren_sync_state_bindings_for_event(list->event_id);
#else
    (void)list;
#endif
}

static void jadren_sync_table_app_state(JadrenNode *node) {
#if JADREN_UI_HAS_FILE_RUNTIME
    JadrenInputAppBinding *binding;
    JadrenTableData *table;
    if (node == NULL) return;
    table = jadren_table_data(node);
    if (table == NULL) return;
    binding = jadren_input_app_binding_for_event(node->event_id);
    if (binding == NULL || binding->kind != JADREN_APP_BIND_TABLE) return;
    (void)app_state_set_int(binding->key, (uint64_t)binding->key_length,
                            (int64_t)table->selected_row);
#else
    (void)node;
#endif
}

/* Keep X11's retained backend equivalent to Win32 at callback boundaries:
 * controls are projections, so every bound native value is published before
 * a different control (such as Save) invokes Jadren code. */
static void jadren_sync_app_bindings_from_native(void) {
#if JADREN_UI_HAS_FILE_RUNTIME
    int index;
    for (index = 1; index < jadren_next_node; index += 1) {
        JadrenNode *node = &jadren_nodes[index];
        JadrenInputAppBinding *binding;
        if (!node->used) continue;
        binding = jadren_input_app_binding_for_event(node->event_id);
        if (binding == NULL) continue;
        switch (node->kind) {
            case JADREN_NODE_INPUT:
                if (binding->kind == JADREN_APP_BIND_TEXT) {
                    jadren_sync_input_app_state(node);
                }
                break;
            case JADREN_NODE_CHECKBOX:
                if (binding->kind == JADREN_APP_BIND_BOOL) {
                    jadren_sync_checkbox_app_state(node);
                }
                break;
            case JADREN_NODE_SELECT:
                if (binding->kind == JADREN_APP_BIND_SELECT) {
                    jadren_sync_select_app_state(node);
                }
                break;
            case JADREN_NODE_LIST:
                if (binding->kind == JADREN_APP_BIND_LIST) {
                    jadren_sync_list_app_state(node);
                }
                break;
            case JADREN_NODE_TABLE:
                if (binding->kind == JADREN_APP_BIND_TABLE) {
                    jadren_sync_table_app_state(node);
                }
                break;
            default:
                break;
        }
        jadren_sync_state_bindings_for_event(node->event_id);
    }
#endif
}

static void jadren_refresh_table_from_app_state(JadrenNode *node) {
#if JADREN_UI_HAS_FILE_RUNTIME
    JadrenInputAppBinding *binding;
    JadrenTableData *table;
    int64_t value;
    if (node == NULL) return;
    table = jadren_table_data(node);
    if (table == NULL) return;
    binding = jadren_input_app_binding_for_event(node->event_id);
    if (binding == NULL || binding->kind != JADREN_APP_BIND_TABLE) return;
    value = app_state_get_int(binding->key, (uint64_t)binding->key_length);
    if (value < -1 || value >= (int64_t)table->row_count) value = -1;
    table->selected_row = (int)value;
    jadren_sync_state_bindings_for_event(node->event_id);
#else
    (void)node;
#endif
}

int ui_list_bind_app_state(int event_id, const char *key_data,
                           uint64_t key_length) {
    JadrenInputAppBinding *binding;
    JadrenNode *list = jadren_node_for_event(event_id, JADREN_NODE_LIST);
    if (list == NULL || key_data == NULL || key_length == 0U ||
        key_length > 64U) {
        return 0;
    }
    binding = jadren_input_app_binding_for_event(event_id);
    if (binding == NULL) {
        if (jadren_input_app_binding_count >= JADREN_X11_MAX_INPUT_APP_BINDINGS) {
            return 0;
        }
        binding = &jadren_input_app_bindings[jadren_input_app_binding_count];
        jadren_input_app_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->kind = JADREN_APP_BIND_LIST;
    binding->key_length = (size_t)key_length;
    jadren_copy_text(binding->key, sizeof(binding->key), key_data, key_length);
#if JADREN_UI_HAS_FILE_RUNTIME
    if (!jadren_preserve_app_binding &&
        !app_state_set_int(binding->key, (uint64_t)binding->key_length,
                           (int64_t)list->list_selected_index)) {
        return 0;
    }
#endif
    if (!jadren_preserve_app_binding) {
        jadren_sync_list_app_state(list);
    }
    return 1;
}

void ui_list_refresh_app_state(int event_id) {
    jadren_refresh_list_from_app_state(
        jadren_node_for_event(event_id, JADREN_NODE_LIST));
}

int ui_table_bind_app_state(int event_id, const char *key_data,
                            uint64_t key_length) {
    JadrenInputAppBinding *binding;
    JadrenNode *table = jadren_node_for_event(event_id, JADREN_NODE_TABLE);
    if (table == NULL || key_data == NULL || key_length == 0U ||
        key_length > 64U) {
        return 0;
    }
    binding = jadren_input_app_binding_for_event(event_id);
    if (binding == NULL) {
        if (jadren_input_app_binding_count >= JADREN_X11_MAX_INPUT_APP_BINDINGS) {
            return 0;
        }
        binding = &jadren_input_app_bindings[jadren_input_app_binding_count];
        jadren_input_app_binding_count += 1;
    }
    binding->event_id = event_id;
    binding->kind = JADREN_APP_BIND_TABLE;
    binding->key_length = (size_t)key_length;
    jadren_copy_text(binding->key, sizeof(binding->key), key_data, key_length);
#if JADREN_UI_HAS_FILE_RUNTIME
    {
        JadrenTableData *table_data = jadren_table_data(table);
        if (table_data == NULL ||
            (!jadren_preserve_app_binding &&
            !app_state_set_int(binding->key, (uint64_t)binding->key_length,
                               (int64_t)table_data->selected_row))) {
            return 0;
        }
    }
#endif
    if (!jadren_preserve_app_binding) {
        jadren_sync_table_app_state(table);
    }
    return 1;
}

void ui_table_refresh_app_state(int event_id) {
    jadren_refresh_table_from_app_state(
        jadren_node_for_event(event_id, JADREN_NODE_TABLE));
}

/* Retained UI convenience binding: callers keep the node id returned by
 * ui_app_* and do not have to repeat the internal event id.  The node kind
 * remains the source of truth, while the existing event-id binding helpers
 * keep one storage/synchronization implementation. */
int ui_app_bind_app_state(int node_id, const char *key_data,
                          uint64_t key_length) {
    JadrenNode *node = jadren_node(node_id);
    int result = 0;
    if (node == NULL || key_data == NULL || key_length == 0U ||
        key_length > 64U) {
        return 0;
    }
    /* ui_app_* is the retained API. A model loaded before the tree is built
     * remains authoritative; explicit event-id binders preserve their legacy
     * initialise-from-control behaviour. */
    jadren_preserve_app_binding = 1;
    switch (node->kind) {
    case JADREN_NODE_INPUT:
        ui_input_bind_app_state(node->event_id, key_data, key_length);
        result = 1;
        break;
    case JADREN_NODE_CHECKBOX:
        result = ui_checkbox_bind_app_state(node->event_id, key_data, key_length);
        break;
    case JADREN_NODE_SELECT:
        result = ui_select_bind_app_state(node->event_id, key_data, key_length);
        break;
    case JADREN_NODE_LIST:
        result = ui_list_bind_app_state(node->event_id, key_data, key_length);
        break;
    case JADREN_NODE_TABLE:
        result = ui_table_bind_app_state(node->event_id, key_data, key_length);
        break;
    default:
        result = 0;
        break;
    }
    jadren_preserve_app_binding = 0;
    if (result) {
        ui_app_refresh_app_state(node_id);
    }
    return result;
}

void ui_app_refresh_app_state(int node_id) {
    JadrenNode *node = jadren_node(node_id);
    if (node == NULL) return;
    switch (node->kind) {
    case JADREN_NODE_INPUT:
        ui_input_refresh_app_state(node->event_id);
        break;
    case JADREN_NODE_CHECKBOX:
        ui_checkbox_refresh_app_state(node->event_id);
        break;
    case JADREN_NODE_SELECT:
        ui_select_refresh_app_state(node->event_id);
        break;
    case JADREN_NODE_LIST:
        ui_list_refresh_app_state(node->event_id);
        break;
    case JADREN_NODE_TABLE:
        ui_table_refresh_app_state(node->event_id);
        break;
    default:
        break;
    }
}

static void jadren_input_backspace(JadrenNode *node) {
    size_t length;
    if (node == NULL) {
        return;
    }
    length = strlen(node->text);
    if (length == 0U) {
        return;
    }
    length -= 1U;
    while (length > 0U &&
           (((unsigned char)node->text[length] & 0xc0U) == 0x80U)) {
        length -= 1U;
    }
    node->text[length] = '\0';
}

static size_t jadren_utf8_fit(const char *data, size_t length,
                              size_t capacity) {
    size_t offset = 0U;
    while (offset < length && offset < capacity) {
        unsigned char lead = (unsigned char)data[offset];
        unsigned char second = 0U;
        size_t width;
        if (lead < 0x80U) {
            width = 1U;
        } else if (lead >= 0xc2U && lead <= 0xdfU) {
            width = 2U;
        } else if (lead >= 0xe0U && lead <= 0xefU) {
            width = 3U;
        } else if (lead >= 0xf0U && lead <= 0xf4U) {
            width = 4U;
        } else {
            /* Xutf8LookupString normally supplies valid UTF-8.  Refuse a
             * malformed byte rather than copying an invalid suffix. */
            break;
        }
        if (width > length - offset || width > capacity - offset) {
            break;
        }
        if (width > 1U) {
            second = (unsigned char)data[offset + 1U];
            if (second < 0x80U || second > 0xbfU) {
                break;
            }
            /* Reject overlong encodings, UTF-16 surrogate code points and
             * values above U+10FFFF while retaining the bounded prefix rule. */
            if ((lead == 0xe0U && second < 0xa0U) ||
                (lead == 0xedU && second > 0x9fU) ||
                (lead == 0xf0U && second < 0x90U) ||
                (lead == 0xf4U && second > 0x8fU)) {
                break;
            }
        }
        if ((width > 2U &&
             ((unsigned char)data[offset + 2U] < 0x80U ||
              (unsigned char)data[offset + 2U] > 0xbfU)) ||
            (width > 3U &&
             ((unsigned char)data[offset + 3U] < 0x80U ||
              (unsigned char)data[offset + 3U] > 0xbfU))) {
            break;
        }
        offset += width;
    }
    return offset;
}

static void jadren_set_focus(int node_id) {
    JadrenNode *node = jadren_node(node_id);
    if (node == NULL || node->kind != JADREN_NODE_INPUT) {
        node_id = 0;
    }
    jadren_focused_node = node_id;
    if (jadren_input_context != NULL) {
        if (node_id != 0) {
            XSetICFocus(jadren_input_context);
        } else {
            XUnsetICFocus(jadren_input_context);
        }
    }
}

static void jadren_open_input_method(void) {
    if (jadren_display == NULL) {
        return;
    }
    /* Xutf8LookupString follows the process locale.  Keep the existing
     * ASCII path available when a minimal host cannot activate a UTF-8
     * locale, but do not ask XIM to interpret bytes under the plain C locale. */
    if (setlocale(LC_CTYPE, "") == NULL) {
        return;
    }
    if (XSetLocaleModifiers("") == NULL) {
        return;
    }
    jadren_input_method = XOpenIM(jadren_display, NULL, NULL, NULL);
    if (jadren_input_method == NULL) {
        return;
    }
    jadren_input_context = XCreateIC(
        jadren_input_method, XNInputStyle,
        (XIMPreeditNothing | XIMStatusNothing), XNClientWindow,
        jadren_window, XNFocusWindow, jadren_window, NULL);
    if (jadren_input_context == NULL) {
        XCloseIM(jadren_input_method);
        jadren_input_method = NULL;
    }
}

static void jadren_close_input_method(void) {
    if (jadren_input_context != NULL) {
        XDestroyIC(jadren_input_context);
        jadren_input_context = NULL;
    }
    if (jadren_input_method != NULL) {
        XCloseIM(jadren_input_method);
        jadren_input_method = NULL;
    }
}

int32_t ui_app_run(void) {
    XEvent event;
    if (jadren_active || jadren_stack_depth != 0) {
        return 2;
    }
    jadren_display = XOpenDisplay(NULL);
    if (jadren_display == NULL) {
        return 3;
    }
    jadren_screen = DefaultScreen(jadren_display);
    jadren_window = XCreateSimpleWindow(
        jadren_display, RootWindow(jadren_display, jadren_screen), 0, 0,
        (unsigned int)jadren_window_width, (unsigned int)jadren_window_height,
        0, BlackPixel(jadren_display, jadren_screen),
        jadren_pixel(jadren_window_background));
    jadren_gc = XCreateGC(jadren_display, jadren_window, 0, NULL);
    jadren_delete_atom = XInternAtom(jadren_display, "WM_DELETE_WINDOW", False);
    XSetWMProtocols(jadren_display, jadren_window, &jadren_delete_atom, 1);
    XStoreName(jadren_display, jadren_window, jadren_window_title);
    jadren_open_input_method();
    XSelectInput(jadren_display, jadren_window,
                 ExposureMask | StructureNotifyMask | ButtonPressMask |
                     PointerMotionMask | LeaveWindowMask | KeyPressMask);
    XMapWindow(jadren_display, jadren_window);
    /* A bare Xvfb has no window manager to assign keyboard focus.  Claim it
     * after mapping so XTest/xdotool and normal native key events reach the
     * retained input just like they do in a managed desktop session. */
    XSetInputFocus(jadren_display, jadren_window, RevertToParent, CurrentTime);
    XFlush(jadren_display);
    for (;;) {
        XNextEvent(jadren_display, &event);
        if (event.type == Expose) {
            jadren_draw();
        } else if (event.type == ConfigureNotify) {
            int requested_width = event.xconfigure.width;
            int requested_height = event.xconfigure.height;
            int corrected = 0;
            jadren_window_width = event.xconfigure.width;
            jadren_window_height = event.xconfigure.height;
            if (jadren_window_width < jadren_window_min_width) {
                jadren_window_width = jadren_window_min_width;
                corrected = 1;
            }
            if (jadren_window_height < jadren_window_min_height) {
                jadren_window_height = jadren_window_min_height;
                corrected = 1;
            }
            if (jadren_window_width > jadren_window_max_width) {
                jadren_window_width = jadren_window_max_width;
                corrected = 1;
            }
            if (jadren_window_height > jadren_window_max_height) {
                jadren_window_height = jadren_window_max_height;
                corrected = 1;
            }
            if (corrected && jadren_window != 0 &&
                (requested_width != jadren_window_width ||
                 requested_height != jadren_window_height)) {
                XResizeWindow(jadren_display, jadren_window,
                              (unsigned int)jadren_window_width,
                              (unsigned int)jadren_window_height);
            }
            if (!corrected && jadren_resize_event_id != 0) {
                jadren_dispatch_event(jadren_resize_event_id);
            }
            if (!corrected) {
                jadren_draw();
            }
        } else if (event.type == MotionNotify) {
            int hit = jadren_hit_test(event.xmotion.x, event.xmotion.y);
            int new_hover = hit > 0 ? hit : 0;
            if (new_hover != jadren_hover_node) {
                jadren_hover_node = new_hover;
                jadren_draw();
            }
        } else if (event.type == LeaveNotify) {
            if (jadren_hover_node != 0) {
                jadren_hover_node = 0;
                jadren_draw();
            }
        } else if (event.type == ButtonPress) {
            XSetInputFocus(jadren_display, jadren_window, RevertToParent,
                           CurrentTime);
            int hit = jadren_hit_test(event.xbutton.x, event.xbutton.y);
            if (hit <= -101) {
                JadrenNode *select = jadren_node(jadren_open_select);
                int option = -hit - 101;
                if (select != NULL && option >= 0 && option < select->option_count) {
                    jadren_open_select = 0;
                    (void)ui_app_select_set_index(select->id, option);
                    jadren_dispatch_event(select->event_id);
                    jadren_draw();
                }
            } else if (hit < 0) {
                JadrenNode *menu = jadren_node(jadren_open_menu);
                int option = -hit - 1;
                if (menu != NULL && option >= 0 && option < menu->menu_item_count) {
                    jadren_open_menu = 0;
                    jadren_dispatch_event(menu->menu_item_events[option]);
                    jadren_draw();
                }
            } else if (hit > 0) {
                JadrenNode *node = jadren_node(hit);
                if (node->kind == JADREN_NODE_INPUT) {
                    jadren_set_focus(hit);
                } else if (node->kind == JADREN_NODE_MENU) {
                    jadren_set_focus(0);
                    jadren_open_menu = hit;
                    jadren_open_select = 0;
                } else if (node->kind == JADREN_NODE_SELECT) {
                    jadren_set_focus(0);
                    jadren_open_select = hit;
                    jadren_open_menu = 0;
                } else if (node->kind == JADREN_NODE_LIST) {
                    jadren_set_focus(0);
                    int row = jadren_list_row_at(node, event.xbutton.x,
                                                 event.xbutton.y);
                    if (row >= 0 && ui_app_list_set_index(node->id, row)) {
                        jadren_dispatch_event(node->event_id);
                    }
                } else if (node->kind == JADREN_NODE_TABLE) {
                    jadren_set_focus(0);
                    int row = jadren_table_row_at(node, event.xbutton.x,
                                                  event.xbutton.y);
                    if (row >= 0 &&
                        ui_app_table_set_selected_row(node->id, row)) {
                        jadren_dispatch_event(node->event_id);
                    }
                } else if (node->kind == JADREN_NODE_CHECKBOX) {
                    jadren_set_focus(0);
                    node->checked = !node->checked;
                    jadren_sync_checkbox_app_state(node);
                    jadren_dispatch_event(node->event_id);
                } else if (node->kind == JADREN_NODE_BUTTON) {
                    jadren_set_focus(0);
                    jadren_dispatch_event(node->event_id);
                }
                jadren_draw();
            } else {
                jadren_set_focus(0);
            }
        } else if (event.type == KeyPress && jadren_focused_node > 0) {
            JadrenNode *node = jadren_node(jadren_focused_node);
            char text[512];
            KeySym key_symbol;
            int count;
            Status lookup_status = 0;
            if (jadren_input_context != NULL) {
                count = Xutf8LookupString(jadren_input_context, &event.xkey,
                                          text, (int)sizeof(text) - 1,
                                          &key_symbol, &lookup_status);
            } else {
                count = XLookupString(&event.xkey, text,
                                      (int)sizeof(text) - 1, &key_symbol, NULL);
            }
            if (node != NULL && node->kind == JADREN_NODE_INPUT) {
                if (key_symbol == XK_BackSpace) {
                    jadren_input_backspace(node);
                    jadren_dispatch_event(node->event_id);
                    jadren_draw();
                } else if (count > 0) {
                    size_t current = strlen(node->text);
                    size_t available = sizeof(node->text) - 1U - current;
                    size_t accepted = jadren_utf8_fit(
                        text, (size_t)count, available);
                    if (lookup_status != XBufferOverflow && accepted > 0U) {
                        memcpy(node->text + current, text, accepted);
                        node->text[current + accepted] = '\0';
                        jadren_dispatch_event(node->event_id);
                        jadren_draw();
                    }
                }
            }
        } else if (event.type == ClientMessage &&
                   (Atom)event.xclient.data.l[0] == jadren_delete_atom) {
            if (!jadren_close_event_sent && jadren_close_event_id != 0) {
                jadren_close_event_sent = 1;
                jadren_dispatch_event(jadren_close_event_id);
            }
            break;
        } else if (event.type == DestroyNotify) {
            if (!jadren_close_event_sent && jadren_close_event_id != 0) {
                jadren_close_event_sent = 1;
                jadren_dispatch_event(jadren_close_event_id);
            }
            break;
        }
    }
    jadren_close_input_method();
    XFreeGC(jadren_display, jadren_gc);
    XDestroyWindow(jadren_display, jadren_window);
    XCloseDisplay(jadren_display);
    jadren_display = NULL;
    return 0;
}

int main(int argc, char **argv) {
#if JADREN_UI_HAS_FILE_RUNTIME
    jadren_process_args_init(argc, argv);
#else
    (void)argc;
    (void)argv;
#endif
    return jadren_entry();
}
