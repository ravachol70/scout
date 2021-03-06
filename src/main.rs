extern crate rustc_hex;
extern crate wasmi;

use rustc_hex::FromHex;
use std::env::args;
use std::fs::File;
use wasmi::memory_units::Pages;
use wasmi::{
    Error as InterpreterError, Externals, FuncInstance, FuncRef, ImportsBuilder, MemoryInstance,
    MemoryRef, Module, ModuleImportResolver, ModuleInstance, NopExternals, RuntimeArgs,
    RuntimeValue, Signature, Trap, ValueType,
};

mod types;
use crate::types::*;

const LOADPRESTATE_FUNC_INDEX: usize = 0;
const BLOCKDATASIZE_FUNC_INDEX: usize = 1;
const BLOCKDATACOPY_FUNC_INDEX: usize = 2;
const SAVEPOSTSTATE_FUNC_INDEX: usize = 3;
const PUSHNEWDEPOSIT_FUNC_INDEX: usize = 4;

struct Runtime<'a> {
    pub memory: Option<MemoryRef>,
    pre_state: &'a Bytes32,
    block_data: &'a ShardBlockBody,
    post_state: Bytes32,
}

impl<'a> Runtime<'a> {
    fn new(pre_state: &'a Bytes32, block_data: &'a ShardBlockBody) -> Runtime<'a> {
        Runtime {
            memory: Some(MemoryInstance::alloc(Pages(1), Some(Pages(1))).unwrap()),
            pre_state: pre_state,
            block_data: block_data,
            post_state: Bytes32::default(),
        }
    }

    fn get_post_state(&self) -> Bytes32 {
        self.post_state
    }
}

impl<'a> Externals for Runtime<'a> {
    fn invoke_index(
        &mut self,
        index: usize,
        args: RuntimeArgs,
    ) -> Result<Option<RuntimeValue>, Trap> {
        match index {
            LOADPRESTATE_FUNC_INDEX => {
                let ptr: u32 = args.nth(0);
                println!("loadprestate to {}", ptr);

                // TODO: add checks for out of bounds access
                let memory = self.memory.as_ref().expect("expects memory");
                memory.set(ptr, &self.pre_state.bytes).unwrap();

                Ok(None)
            }
            SAVEPOSTSTATE_FUNC_INDEX => {
                let ptr: u32 = args.nth(0);
                println!("savepoststate from {}", ptr);

                // TODO: add checks for out of bounds access
                let memory = self.memory.as_ref().expect("expects memory");
                memory.get_into(ptr, &mut self.post_state.bytes).unwrap();

                Ok(None)
            }
            BLOCKDATASIZE_FUNC_INDEX => {
                let ret: i32 = self.block_data.data.len() as i32;
                println!("blockdatasize {}", ret);
                Ok(Some(ret.into()))
            }
            BLOCKDATACOPY_FUNC_INDEX => {
                let ptr: u32 = args.nth(0);
                let offset: u32 = args.nth(1);
                let length: u32 = args.nth(2);
                println!(
                    "blockdatacopy to {} from {} for {} bytes",
                    ptr, offset, length
                );

                // TODO: add overflow check
                let offset = offset as usize;
                let length = length as usize;

                // TODO: add checks for out of bounds access
                let memory = self.memory.as_ref().expect("expects memory");
                memory
                    .set(ptr, &self.block_data.data[offset..length])
                    .unwrap();

                Ok(None)
            }
            PUSHNEWDEPOSIT_FUNC_INDEX => unimplemented!(),
            _ => panic!("unknown function index"),
        }
    }
}

struct RuntimeModuleImportResolver;

impl<'a> ModuleImportResolver for RuntimeModuleImportResolver {
    fn resolve_func(
        &self,
        field_name: &str,
        _signature: &Signature,
    ) -> Result<FuncRef, InterpreterError> {
        let func_ref = match field_name {
            "eth2_loadPreState" => FuncInstance::alloc_host(
                Signature::new(&[ValueType::I32][..], None),
                LOADPRESTATE_FUNC_INDEX,
            ),
            "eth2_blockDataSize" => FuncInstance::alloc_host(
                Signature::new(&[][..], Some(ValueType::I32)),
                BLOCKDATASIZE_FUNC_INDEX,
            ),
            "eth2_blockDataCopy" => FuncInstance::alloc_host(
                Signature::new(&[ValueType::I32, ValueType::I32, ValueType::I32][..], None),
                BLOCKDATACOPY_FUNC_INDEX,
            ),
            "eth2_savePostState" => FuncInstance::alloc_host(
                Signature::new(&[ValueType::I32][..], None),
                SAVEPOSTSTATE_FUNC_INDEX,
            ),
            "eth2_pushNewDeposit" => FuncInstance::alloc_host(
                Signature::new(&[ValueType::I32][..], None),
                PUSHNEWDEPOSIT_FUNC_INDEX,
            ),
            _ => {
                return Err(InterpreterError::Function(format!(
                    "host module doesn't export function with name {}",
                    field_name
                )))
            }
        };
        Ok(func_ref)
    }
}

fn wasm_load_from_file(filename: &str) -> Module {
    use std::io::prelude::*;
    let mut file = File::open(filename).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    Module::from_buffer(buf).unwrap()
}

fn wasm_load_from_blob(buf: &[u8]) -> Module {
    Module::from_buffer(buf).unwrap()
}

const BYTES_PER_SHARD_BLOCK_BODY: usize = 16384;
const ZERO_HASH: Bytes32 = Bytes32 { bytes: [0u8; 32] };

/// These are Phase 0 structures.
/// https://github.com/ethereum/eth2.0-specs/blob/dev/specs/core/0_beacon-chain.md
#[derive(Default, Clone, Debug)]
pub struct Deposit {}

/// These are Phase 2 Proposal 2 structures.

#[derive(Default, Clone, Debug)]
pub struct ExecutionScript {
    code: Vec<u8>,
}

#[derive(Default, Clone, Debug)]
pub struct BeaconState {
    execution_scripts: Vec<ExecutionScript>,
}

/// Shards are Phase 1 structures.
/// https://github.com/ethereum/eth2.0-specs/blob/dev/specs/core/1_shard-data-chains.md

#[derive(Default, Clone, Debug)]
pub struct ShardBlockHeader {}

#[derive(Default, Clone, Debug)]
pub struct ShardBlockBody {
    data: Vec<u8>,
}

#[derive(Default, Clone, Debug)]
pub struct ShardBlock {
    env: u64, // This is added by Phase 2 Proposal 2
    data: ShardBlockBody,
    // TODO: add missing fields
}

#[derive(Default, Clone, Debug)]
pub struct ShardState {
    exec_env_states: Vec<Bytes32>,
    slot: u64,
    parent_block: ShardBlockHeader,
    // TODO: add missing field
    // latest_state_roots: [bytes32, LATEST_STATE_ROOTS_LEMGTH]
}

pub fn execute_code(
    code: &[u8],
    pre_state: &Bytes32,
    block_data: &ShardBlockBody,
) -> (Bytes32, Vec<Deposit>) {
    println!(
        "Executing codesize({}) and data: {:#?}",
        code.len(),
        block_data
    );

    let module = wasm_load_from_blob(&code);
    let mut imports = ImportsBuilder::new();
    // FIXME: use eth2
    imports.push_resolver("env", &RuntimeModuleImportResolver);

    let instance = ModuleInstance::new(&module, &imports)
        .unwrap()
        .assert_no_start();

    let mut runtime = Runtime::new(pre_state, block_data);

    let internal_mem = instance
        .export_by_name("memory")
        .expect("Module expected to have 'memory' export")
        .as_memory()
        .cloned()
        .expect("'memory' export should be a memory");

    runtime.memory = Some(internal_mem);

    let result = instance
        .invoke_export("main", &[], &mut runtime)
        .expect("Executed 'main'");

    println!("Result: {:?}", result);
    println!("Execution finished");

    (runtime.get_post_state(), vec![Deposit {}])
}

pub fn process_shard_block(
    state: &mut ShardState,
    beacon_state: BeaconState,
    block: Option<ShardBlock>,
) {
    // println!("Beacon state: {:#?}", beacon_state);
    println!("Executing block: {:#?}", block);

    println!("Pre-execution: {:#?}", state);

    // TODO: implement state root handling

    if let Some(block) = block {
        // The execution environment identifier
        let env = block.env as usize; // FIXME: usize can be 32-bit
        let code = &beacon_state.execution_scripts[env].code;

        // Set post states to empty for any holes
        for x in 0..env {
            state.exec_env_states.push(ZERO_HASH)
        }
        let pre_state = &state.exec_env_states[env];
        let (post_state, deposits) = execute_code(code, pre_state, &block.data);
        state.exec_env_states[env] = post_state
    }

    // TODO: implement state + deposit root handling

    println!("Post-execution: {:#?}", state)
}

fn load_file(filename: &str) -> Vec<u8> {
    use std::io::prelude::*;
    let mut file = File::open(filename).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    buf
}

fn main() {
    let execution_script = load_file("phase2_helloworld.wasm");

    let mut shard_state = ShardState {
        exec_env_states: vec![Bytes32::default()],
        slot: 0,
        parent_block: ShardBlockHeader {},
    };
    let beacon_state = BeaconState {
        execution_scripts: vec![
            ExecutionScript {
                code: execution_script.to_vec(),
            },
            ExecutionScript {
                code: execution_script.to_vec(),
            },
        ],
    };
    let shard_block = ShardBlock {
        env: 1,
        data: ShardBlockBody { data: vec![] },
    };
    process_shard_block(&mut shard_state, beacon_state, Some(shard_block))
}
