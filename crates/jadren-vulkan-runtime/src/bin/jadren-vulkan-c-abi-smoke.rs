#![allow(unsafe_code)]

// JADREN-UNSAFE-AUDIT: this binary is a tiny C-ABI probe; raw pointers are
// fixed local arrays and are passed only for the duration of the smoke call.

use jadren_vulkan_runtime::{
    jadren_vk_f32_add_one, jadren_vk_f32_add_one_array, jadren_vk_u32_3d_strided_write,
    jadren_vk_u32_3d_strided_write_async, jadren_vk_u32_3d_strided_write_async_complete,
    jadren_vk_u32_3d_strided_write_async_poll, jadren_vk_u32_3d_strided_write_async_release,
    jadren_vk_u32_add_one_array, jadren_vk_u32_binary_array,
};
use serde::Serialize;

#[derive(Serialize)]
struct CAbiSmokeReport {
    schema: &'static str,
    status: i32,
    output_value: f32,
    physical_device_count: u32,
    f32_array_status: i32,
    f32_array_output_checksum: f64,
    f32_array_processed_length: u32,
    f32_array_output: [f32; 4],
    u32_status: i32,
    u32_output_checksum: u64,
    u32_processed_length: u32,
    u32_output: [u32; 4],
    binary_status: i32,
    binary_output_checksum: u64,
    binary_processed_length: u32,
    binary_operation: u32,
    binary_operand: u32,
    binary_output: [u32; 4],
    tensor3d_status: i32,
    tensor3d_output_checksum: u64,
    tensor3d_timeline_value: u64,
    tensor3d_timeline_completed: u32,
    tensor3d_last_physical_index: u32,
    tensor3d_written_elements: u32,
    tensor3d_untouched_elements: u32,
    tensor3d_output: [u32; 32],
    tensor3d_async_status: i32,
    tensor3d_async_output_checksum: u64,
    tensor3d_async_timeline_value: u64,
    tensor3d_async_timeline_completed: u32,
    tensor3d_async_untouched_elements: u32,
    tensor3d_async_output: [u32; 32],
    tensor3d_async_release_status: i32,
}

fn main() {
    let input = [0.0_f32, 41.0_f32];
    let mut output = [0.0_f32; 2];
    let result = unsafe { jadren_vk_f32_add_one(input.as_ptr(), output.as_mut_ptr(), 2) };
    if result.status != 0 || output[1] != 42.0_f32 || result.output_value != 42.0_f32 {
        eprintln!("native Vulkan C ABI smoke failed: {result:?}, output={output:?}");
        std::process::exit(1);
    }
    let f32_array_input = [7.0_f32, 10.0, 13.0, 16.0];
    let mut f32_array_output = [0.0_f32; 4];
    let f32_array_result = unsafe {
        jadren_vk_f32_add_one_array(
            f32_array_input.as_ptr(),
            f32_array_output.as_mut_ptr(),
            f32_array_input.len() as u32,
        )
    };
    let f32_array_expected = [8.0_f32, 11.0, 14.0, 17.0];
    if f32_array_result.status != 0
        || f32_array_output != f32_array_expected
        || f32_array_result.output_checksum != 50.0
        || f32_array_result.processed_length != f32_array_expected.len() as u32
    {
        eprintln!(
            "native Vulkan f32 array C ABI smoke failed: {f32_array_result:?}, output={f32_array_output:?}"
        );
        std::process::exit(1);
    }
    let u32_input = [7_u32, 10, 13, 16];
    let mut u32_output = [0_u32; 4];
    let u32_result = unsafe {
        jadren_vk_u32_add_one_array(
            u32_input.as_ptr(),
            u32_output.as_mut_ptr(),
            u32_input.len() as u32,
        )
    };
    let expected = [8_u32, 11, 14, 17];
    if u32_result.status != 0
        || u32_output != expected
        || u32_result.output_checksum != expected.iter().map(|value| u64::from(*value)).sum::<u64>()
        || u32_result.processed_length != expected.len() as u32
    {
        eprintln!(
            "native Vulkan u32 array C ABI smoke failed: {u32_result:?}, output={u32_output:?}"
        );
        std::process::exit(1);
    }
    let binary_input = [1_u32, 2, 3, 4];
    let mut binary_output = [0_u32; 4];
    let binary_result = unsafe {
        jadren_vk_u32_binary_array(
            binary_input.as_ptr(),
            binary_output.as_mut_ptr(),
            binary_input.len() as u32,
            2,
            3,
        )
    };
    let binary_expected = [3_u32, 6, 9, 12];
    if binary_result.status != 0
        || binary_output != binary_expected
        || binary_result.output_checksum != 30
        || binary_result.processed_length != 4
        || binary_result.operation != 2
        || binary_result.operand != 3
    {
        eprintln!(
            "native Vulkan u32 binary C ABI smoke failed: {binary_result:?}, output={binary_output:?}"
        );
        std::process::exit(1);
    }
    let mut tensor3d_output = [0_u32; 32];
    let tensor3d_result = unsafe {
        jadren_vk_u32_3d_strided_write(
            tensor3d_output.as_mut_ptr(),
            2,
            2,
            2,
            1,
            5,
            13,
            tensor3d_output.len() as u32,
            7,
        )
    };
    let mut tensor3d_expected = [0_u32; 32];
    for z in 0..2_usize {
        for y in 0..2_usize {
            for x in 0..2_usize {
                tensor3d_expected[x + y * 5 + z * 13] = 7;
            }
        }
    }
    let tensor3d_checksum = tensor3d_expected
        .iter()
        .map(|value| u64::from(*value))
        .sum::<u64>();
    if tensor3d_result.status != 0
        || tensor3d_output != tensor3d_expected
        || tensor3d_result.output_checksum != tensor3d_checksum
        || tensor3d_result.timeline_value != 1
        || tensor3d_result.timeline_completed != 1
        || tensor3d_result.last_physical_index != 19
        || tensor3d_result.written_elements != 8
        || tensor3d_result.untouched_elements != 24
    {
        eprintln!(
            "native Vulkan 3D affine C ABI smoke failed: {tensor3d_result:?}, output={tensor3d_output:?}"
        );
        std::process::exit(1);
    }
    let mut tensor3d_async_output = [0_u32; 32];
    let async_begin = unsafe {
        jadren_vk_u32_3d_strided_write_async(
            tensor3d_async_output.as_mut_ptr(),
            2,
            2,
            2,
            1,
            5,
            13,
            tensor3d_async_output.len() as u32,
            7,
        )
    };
    if async_begin.status != 0 || async_begin.handle.is_null() {
        eprintln!("native Vulkan 3D async C ABI begin failed: {async_begin:?}");
        std::process::exit(1);
    }
    loop {
        let poll = unsafe { jadren_vk_u32_3d_strided_write_async_poll(async_begin.handle) };
        if poll < 0 {
            eprintln!("native Vulkan 3D async C ABI poll failed: {poll}");
            std::process::exit(1);
        }
        if poll == 1 {
            break;
        }
        std::thread::yield_now();
    }
    let tensor3d_async_result =
        unsafe { jadren_vk_u32_3d_strided_write_async_complete(async_begin.handle) };
    if tensor3d_async_result.status != 0
        || tensor3d_async_output != tensor3d_expected
        || tensor3d_async_result.output_checksum != tensor3d_checksum
        || tensor3d_async_result.timeline_value != 1
        || tensor3d_async_result.timeline_completed != 1
        || tensor3d_async_result.untouched_elements != 24
    {
        eprintln!(
            "native Vulkan 3D async C ABI smoke failed: {tensor3d_async_result:?}, output={tensor3d_async_output:?}"
        );
        std::process::exit(1);
    }
    let mut tensor3d_release_output = [0_u32; 32];
    let release_begin = unsafe {
        jadren_vk_u32_3d_strided_write_async(
            tensor3d_release_output.as_mut_ptr(),
            2,
            2,
            2,
            1,
            5,
            13,
            tensor3d_release_output.len() as u32,
            7,
        )
    };
    if release_begin.status != 0 || release_begin.handle.is_null() {
        eprintln!("native Vulkan 3D async C ABI release begin failed: {release_begin:?}");
        std::process::exit(1);
    }
    let tensor3d_async_release_status =
        unsafe { jadren_vk_u32_3d_strided_write_async_release(release_begin.handle) };
    if tensor3d_async_release_status != 0 {
        eprintln!("native Vulkan 3D async C ABI release failed: {tensor3d_async_release_status}");
        std::process::exit(1);
    }
    let report = CAbiSmokeReport {
        schema: "jadren-vulkan-c-abi-smoke-0.1",
        status: result.status,
        output_value: result.output_value,
        physical_device_count: result.physical_device_count,
        f32_array_status: f32_array_result.status,
        f32_array_output_checksum: f32_array_result.output_checksum,
        f32_array_processed_length: f32_array_result.processed_length,
        f32_array_output,
        u32_status: u32_result.status,
        u32_output_checksum: u32_result.output_checksum,
        u32_processed_length: u32_result.processed_length,
        u32_output,
        binary_status: binary_result.status,
        binary_output_checksum: binary_result.output_checksum,
        binary_processed_length: binary_result.processed_length,
        binary_operation: binary_result.operation,
        binary_operand: binary_result.operand,
        binary_output,
        tensor3d_status: tensor3d_result.status,
        tensor3d_output_checksum: tensor3d_result.output_checksum,
        tensor3d_timeline_value: tensor3d_result.timeline_value,
        tensor3d_timeline_completed: tensor3d_result.timeline_completed,
        tensor3d_last_physical_index: tensor3d_result.last_physical_index,
        tensor3d_written_elements: tensor3d_result.written_elements,
        tensor3d_untouched_elements: tensor3d_result.untouched_elements,
        tensor3d_output,
        tensor3d_async_status: tensor3d_async_result.status,
        tensor3d_async_output_checksum: tensor3d_async_result.output_checksum,
        tensor3d_async_timeline_value: tensor3d_async_result.timeline_value,
        tensor3d_async_timeline_completed: tensor3d_async_result.timeline_completed,
        tensor3d_async_untouched_elements: tensor3d_async_result.untouched_elements,
        tensor3d_async_output,
        tensor3d_async_release_status,
    };
    match serde_json::to_string_pretty(&report) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("failed to serialize native Vulkan C ABI report: {error}");
            std::process::exit(1);
        }
    }
}
