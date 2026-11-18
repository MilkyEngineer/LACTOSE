// The Metal step of `cargo xtask gpu` (tools/xtask/README.md, "GPU runs"), which the `shaders`
// CI jobs run on each macOS runner: every cooked shader in a directory that
// `ricotta --code` wrote, compiled or loaded through Metal and made into a pipeline state with
// its function constants.
//
//   swift tools/xtask/metal/pipelines.swift <code directory>
//   swift tools/xtask/metal/pipelines.swift --canary api|shader
//   swift tools/xtask/metal/pipelines.swift --device
//
// Each file is `<source>.<entry>.<target>`. An `.msl` file is compiled at run time as MSL
// 3.1, named in the options since their default is the system's newest; a `.metallib` file
// is loaded as it is, and needs the `.msl` of the same entry beside it, which says its
// stage, its function constants and its colours. A source's fragment entries are each paired
// with its one vertex entry, a vertex entry with no fragment beside it makes a depth-only
// pipeline, and a kernel makes a compute pipeline. Every function constant is given the
// zero of its type. The device is the first of `MTLCopyAllDevices`; with none, the step
// fails. Its facts are printed first: its name, its argument buffers tier, its highest Apple
// GPU family and whether it is of the Metal 3 family. A directory whose code takes the heap
// needs tier 2, for the heap's 1024 textures in one argument buffer, and one whose fragment
// code reads a colour needs an Apple GPU family, for framebuffer fetch; the step fails naming
// each one the device lacks before it makes any pipeline state. `--device` prints the facts
// alone. Prints one line per pipeline state to standard output and exits 1 at the first
// failure, naming it on standard error.

import Foundation
import Metal

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("pipelines: \(message)\n".utf8))
    exit(1)
}

enum Stage { case vertex, fragment, kernel }

/// What the MSL says of one entry point.
struct Entry {
    let source: String
    let name: String
    let stage: Stage
    let constants: [(index: Int, type: MTLDataType)]
    /// The colour attachments it reads or writes, with the format the step gives each.
    let colours: [Int: MTLPixelFormat]
    /// Whether it takes the heap's argument buffer, whose type Slang names `Heap_default_`.
    let heap: Bool
    /// Whether it reads a colour attachment, which Metal does through framebuffer fetch.
    let readsColour: Bool
}

/// Each `[[<attribute>(<index>)]]` of `text`, with the type written first on its line or
/// before it in its parameter.
func attributes(_ text: Substring, _ attribute: String) -> [(index: Int, type: String)] {
    var found: [(Int, String)] = []
    for line in text.split(separator: "\n") {
        for part in line.split(separator: ",") {
            guard let open = part.range(of: "[[\(attribute)(") else { continue }
            let rest = part[open.upperBound...]
            guard let close = rest.firstIndex(of: ")"), let index = Int(rest[..<close]) else {
                fail("an attribute that does not read: \(line)")
            }
            let words = part[..<open.lowerBound].split(whereSeparator: { $0 == " " || $0 == "(" })
            let type = words.count >= 2 ? String(words[words.count - 2]) : ""
            let constant = words.first == "constant" && words.count >= 3 ? String(words[1]) : type
            found.append((index, constant))
        }
    }
    return found
}

/// The `MTLDataType` of a function constant's MSL type, which is its type in the cooked format:
/// the cook refuses an MSL whose constants' types are not the format's `ConstantType`s
/// (tools/cook/README.md, "The checks"). The step reads them from the MSL since it is given
/// the code `ricotta --code` writes, not the pack the format is encoded in.
func dataType(_ type: String) -> MTLDataType {
    switch type {
    case "bool": return .bool
    case "int": return .int
    case "uint": return .uint
    case "float": return .float
    default: fail("a function constant of type \(type), which the step does not give a value")
    }
}

func pixelFormat(_ type: String) -> MTLPixelFormat {
    switch type {
    case "float", "half": return .r16Float
    case "float2", "half2": return .rg16Float
    case "float4", "half4": return .rgba16Float
    case "uint": return .r32Uint
    case "uint2": return .rg32Uint
    case "uint4": return .rgba32Uint
    case "int": return .r32Sint
    case "int2": return .rg32Sint
    case "int4": return .rgba32Sint
    default: fail("a colour of type \(type), which the step gives no format")
    }
}

func read(source: String, name: String, msl: String) -> Entry {
    let qualifiers: [(String, Stage)] = [
        ("[[vertex]] ", .vertex), ("[[fragment]] ", .fragment), ("[[kernel]] ", .kernel),
    ]
    let lines = msl.split(separator: "\n")
    var declaration: (Substring, Stage, String)? = nil
    for line in lines {
        for (qualifier, stage) in qualifiers where line.hasPrefix(qualifier) {
            if let call = line.range(of: " \(name)(") {
                let returned = line[line.index(line.startIndex, offsetBy: qualifier.count)..<call.lowerBound]
                declaration = (line[call.upperBound...], stage, String(returned))
            }
        }
    }
    guard let (parameters, stage, returned) = declaration else {
        fail("\(source).\(name): the MSL declares no entry point \(name)")
    }
    let constants = attributes(Substring(msl), "function_constant").map { ($0.index, dataType($0.type)) }
    var colours: [Int: MTLPixelFormat] = [:]
    let inputs = stage == .fragment ? attributes(parameters, "color") : []
    if stage == .fragment {
        for (index, type) in inputs { colours[index] = pixelFormat(type) }
        if returned != "void" {
            guard let header = msl.range(of: "struct \(returned)\n{\n"),
                  let end = msl[header.upperBound...].range(of: "\n};") else {
                fail("\(source).\(name): the MSL does not declare the struct \(returned)")
            }
            for (index, type) in attributes(msl[header.upperBound..<end.lowerBound], "color") {
                colours[index] = pixelFormat(type)
            }
        }
    }
    let heap = parameters.split(separator: ",").contains { $0.trimmingCharacters(in: .whitespaces).hasPrefix("Heap_default_") }
    return Entry(
        source: source, name: name, stage: stage, constants: constants, colours: colours,
        heap: heap, readsColour: !inputs.isEmpty)
}

func function(_ library: MTLLibrary, _ entry: Entry) -> MTLFunction {
    let values = MTLFunctionConstantValues()
    var zero = [UInt8](repeating: 0, count: 16)
    for constant in entry.constants {
        values.setConstantValue(&zero, type: constant.type, index: constant.index)
    }
    do {
        return try library.makeFunction(name: entry.name, constantValues: values)
    } catch {
        fail("\(entry.source).\(entry.name): no function with its constants: \(error)")
    }
}

let arguments = CommandLine.arguments
let usage = "usage: swift pipelines.swift <code directory> | --canary api | --canary shader | --device"
guard arguments.count == 2 || (arguments.count == 3 && arguments[1] == "--canary") else {
    fail(usage)
}

guard let device = MTLCopyAllDevices().first else {
    fail("no Metal device: MTLCopyAllDevices returned none")
}
let tier2 = device.argumentBuffersSupport == .tier2
// The highest of `MTLGPUFamily.apple1` to `apple10`, by raw value, so an SDK that lacks the
// newest names still builds: 0 for none.
let appleFamily = (1...10).last { family in
    MTLGPUFamily(rawValue: 1000 + family).map { device.supportsFamily($0) } ?? false
} ?? 0
let metal3 = device.supportsFamily(.metal3)
print("device: \(device.name), argument buffers tier \(tier2 ? 2 : 1), Apple family \(appleFamily == 0 ? "none" : String(appleFamily)), Metal 3 \(metal3 ? "yes" : "no")")
if arguments[1] == "--device" { exit(0) }
print("system: \(ProcessInfo.processInfo.operatingSystemVersionString)")

// The validators' canaries (milestone 2 plan, decision 5.10), one per process. `api` binds no
// buffer at index 0, which Metal API validation asserts on and aborts with `missing Buffer
// binding at index 0`. `shader` stores four megabytes past the end of a four-byte buffer,
// which Metal shader validation reports with `Invalid store in 'device' or 'constant' address
// space`. With neither validator listening, either completes, and the caller fails the run.
if arguments.count == 3 {
    let canary = arguments[2]
    guard canary == "api" || canary == "shader" else { fail(usage) }
    let source = """
        kernel void canary(device uint* o [[buffer(0)]], constant uint& at [[buffer(1)]]) {
            o[at] = 1;
        }
        """
    guard let library = try? device.makeLibrary(source: source, options: nil),
          let kernel = library.makeFunction(name: "canary"),
          let pipeline = try? device.makeComputePipelineState(function: kernel),
          let queue = device.makeCommandQueue(),
          let commands = queue.makeCommandBuffer(),
          let encoder = commands.makeComputeCommandEncoder(),
          let buffer = device.makeBuffer(length: 4, options: .storageModeShared) else {
        fail("the canary's kernel is not made")
    }
    encoder.setComputePipelineState(pipeline)
    var at: UInt32 = canary == "api" ? 0 : 1 << 20
    if canary == "shader" { encoder.setBuffer(buffer, offset: 0, index: 0) }
    encoder.setBytes(&at, length: 4, index: 1)
    let one = MTLSize(width: 1, height: 1, depth: 1)
    encoder.dispatchThreads(one, threadsPerThreadgroup: one)
    encoder.endEncoding()
    commands.commit()
    commands.waitUntilCompleted()
    print("canary \(canary): completed")
    exit(0)
}
let directory = URL(fileURLWithPath: arguments[1], isDirectory: true)

guard let walk = FileManager.default.enumerator(at: directory, includingPropertiesForKeys: nil) else {
    fail("cannot list \(directory.path)")
}
var files: [String: URL] = [:]
for case let url as URL in walk where !url.hasDirectoryPath {
    let relative = String(url.standardizedFileURL.path.dropFirst(directory.standardizedFileURL.path.count + 1))
    files[relative] = url
}

/// Each entry by target, `msl` and `metallib`, with its library.
var entries: [String: [(Entry, MTLLibrary)]] = ["msl": [], "metallib": []]
let options = MTLCompileOptions()
options.languageVersion = .version3_1
for name in files.keys.sorted() {
    let parts = name.split(separator: ".", omittingEmptySubsequences: false)
    guard parts.count >= 3 else { fail("\(name): not <source>.<entry>.<target>") }
    let target = String(parts[parts.count - 1])
    let entryName = String(parts[parts.count - 2])
    let source = parts[..<(parts.count - 2)].joined(separator: ".")
    if target == "spirv" { continue }
    guard target == "msl" || target == "metallib" else { fail("\(name): an unknown target") }
    let mslName = "\(source).\(entryName).msl"
    guard let mslFile = files[mslName], let msl = try? String(contentsOf: mslFile, encoding: .utf8) else {
        fail("\(name): no readable \(mslName) beside it")
    }
    let entry = read(source: source, name: entryName, msl: msl)
    let library: MTLLibrary
    do {
        library = target == "msl"
            ? try device.makeLibrary(source: msl, options: options)
            : try device.makeLibrary(URL: files[name]!)
    } catch {
        fail("\(name): Metal refuses it: \(error)")
    }
    entries[target]!.append((entry, library))
}

// What the code needs of the device, each one it lacks named before any pipeline is made.
let cooked = entries.values.flatMap { $0.map { $0.0 } }
var lacking: [String] = []
if let entry = cooked.first(where: { $0.heap }), !tier2 {
    lacking.append("argument buffers tier 2, for the heap's argument buffer that \(entry.source).\(entry.name) takes")
}
if let entry = cooked.first(where: { $0.readsColour }), appleFamily == 0 {
    lacking.append("an Apple GPU family, for the framebuffer fetch of the colour that \(entry.source).\(entry.name) reads")
}
if !lacking.isEmpty { fail("the device lacks " + lacking.joined(separator: "; and ")) }

var made = 0
for target in ["msl", "metallib"] {
    let all = entries[target]!
    for source in Set(all.map { $0.0.source }).sorted() {
        let mine = all.filter { $0.0.source == source }
        let vertices = mine.filter { $0.0.stage == .vertex }
        let fragments = mine.filter { $0.0.stage == .fragment }
        for (entry, library) in mine where entry.stage == .kernel {
            do {
                _ = try device.makeComputePipelineState(function: function(library, entry))
            } catch {
                fail("\(target) \(source).\(entry.name): no compute pipeline state: \(error)")
            }
            print("\(target) \(source) \(entry.name) ok")
            made += 1
        }
        if !fragments.isEmpty && vertices.count != 1 {
            fail("\(target) \(source): \(vertices.count) vertex entries beside its fragment entries, where one pairs with them")
        }
        let pairs: [((Entry, MTLLibrary), (Entry, MTLLibrary)?)] = fragments.isEmpty
            ? vertices.map { ($0, nil) }
            : fragments.map { (vertices[0], $0) }
        for (vertex, fragment) in pairs {
            let descriptor = MTLRenderPipelineDescriptor()
            descriptor.vertexFunction = function(vertex.1, vertex.0)
            var label = vertex.0.name
            if let (entry, library) = fragment {
                descriptor.fragmentFunction = function(library, entry)
                for (index, format) in entry.colours { descriptor.colorAttachments[index].pixelFormat = format }
                label += "+\(entry.name)"
            } else {
                descriptor.depthAttachmentPixelFormat = .depth32Float
            }
            do {
                _ = try device.makeRenderPipelineState(descriptor: descriptor)
            } catch {
                fail("\(target) \(source) \(label): no render pipeline state: \(error)")
            }
            print("\(target) \(source) \(label) ok")
            made += 1
        }
    }
}
if made == 0 { fail("no cooked MSL or metallib in \(directory.path)") }
print("pipelines: \(made)")
