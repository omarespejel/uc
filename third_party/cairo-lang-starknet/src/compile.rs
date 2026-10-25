use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cairo_lang_compiler::db::RootDatabase;
use cairo_lang_compiler::project::setup_project;
use cairo_lang_compiler::{CompilerConfig, ensure_diagnostics};
use cairo_lang_defs::db::DefsGroup;
use cairo_lang_defs::ids::{
    FunctionWithBodyId as DefsFunctionWithBodyId, ModuleId, ModuleItemId, TopLevelLanguageElementId,
};
use cairo_lang_diagnostics::ToOption;
use cairo_lang_filesystem::ids::{CrateId, CrateInput};
use cairo_lang_lowering::db::LoweringGroup;
use cairo_lang_lowering::ids::ConcreteFunctionWithBodyId;
use cairo_lang_lowering::optimizations::config::Optimizations;
use cairo_lang_lowering::utils::InliningStrategy;
use cairo_lang_parser::db::ParserGroup;
use cairo_lang_semantic::db::SemanticGroup;
use cairo_lang_semantic::items::enm::EnumSemantic;
use cairo_lang_semantic::items::free_function::FreeFunctionSemantic;
use cairo_lang_semantic::items::imp::ImplSemantic;
use cairo_lang_semantic::items::structure::StructSemantic;
use cairo_lang_semantic::items::trt::TraitSemantic;
use cairo_lang_sierra::debug_info::Annotations;
use cairo_lang_sierra_generator::canonical_id_replacer::CanonicalReplacer;
use cairo_lang_sierra_generator::db::SierraGenGroup;
use cairo_lang_sierra_generator::program_generator::SierraProgramWithDebug;
use cairo_lang_sierra_generator::replace_ids::{SierraIdReplacer, replace_sierra_ids_in_program};
use cairo_lang_starknet_classes::allowed_libfuncs::ListSelector;
use cairo_lang_starknet_classes::contract_class::{
    ContractClass, ContractEntryPoint, ContractEntryPoints,
};
use cairo_lang_utils::CloneableDatabase;
use itertools::{Itertools, chain};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use salsa::Database;

use crate::abi::AbiBuilder;
use crate::aliased::Aliased;
use crate::contract::{
    ContractDeclaration, find_contracts, get_contract_abi_functions,
    get_selector_and_sierra_function,
};
use crate::plugin::consts::{CONSTRUCTOR_MODULE, EXTERNAL_MODULE, L1_HANDLER_MODULE};
use crate::starknet_plugin_suite;

#[cfg(test)]
#[path = "compile_test.rs"]
mod test;

/// Compile the contract given by the path.
/// Errors if there is ambiguity.
pub fn compile_path(
    path: &Path,
    contract_path: Option<&str>,
    mut compiler_config: CompilerConfig<'_>,
    inlining_strategy: InliningStrategy,
) -> Result<ContractClass> {
    let mut db = RootDatabase::builder()
        .with_optimizations(Optimizations::enabled_with_default_movable_functions(
            inlining_strategy,
        ))
        .detect_corelib()
        .with_default_plugin_suite(starknet_plugin_suite())
        .build()?;

    let main_crate_inputs = setup_project(&mut db, path)?;
    compiler_config.diagnostics_reporter =
        compiler_config.diagnostics_reporter.with_crates(&main_crate_inputs);
    let main_crate_ids = CrateInput::into_crate_ids(&db, main_crate_inputs);
    compile_contract_in_prepared_db(&db, contract_path, main_crate_ids, compiler_config)
}

/// Runs Starknet contract compiler on the specified contract.
/// If no contract was specified, verify that there is only one.
/// Otherwise, returns an error.
pub fn compile_contract_in_prepared_db<'db>(
    db: &'db dyn CloneableDatabase,
    contract_path: Option<&str>,
    main_crate_ids: Vec<CrateId<'db>>,
    mut compiler_config: CompilerConfig<'_>,
) -> Result<ContractClass> {
    let contracts = find_contracts(db, &main_crate_ids);
    if contracts.is_empty() {
        // Report diagnostics as they might reveal the reason why no contract was found.
        compiler_config.diagnostics_reporter.ensure(db)?;
        anyhow::bail!("No contracts found.");
    }
    let name = |contract: &ContractDeclaration<'db>| contract.submodule_id.full_path(db);
    let contract = if let Some(contract_path) = contract_path {
        if let Some(contract) = contracts.iter().find(|contract| name(contract) == contract_path) {
            contract
        } else {
            // Report diagnostics as they might reveal the reason why the contract was not found.
            compiler_config.diagnostics_reporter.ensure(db)?;
            anyhow::bail!(
                "No contract matching `--contract-path={contract_path}`, available contracts \
                 are:\n  {}",
                contracts.iter().map(name).join("\n  ")
            );
        }
    } else if let [contract] = contracts.as_slice() {
        contract
    } else {
        anyhow::bail!(
            "More than one contract found in the main crate:\n  {}\nUse --contract-path to \
             specify which to compile.",
            contracts.iter().map(name).join("\n  ")
        );
    };

    Ok(compile_prepared_db(db, &[contract], compiler_config)?.into_iter().exactly_one()?)
}

/// Runs Starknet contract compiler.
///
/// # Arguments
/// * `db` - Preloaded compilation database.
/// * `contracts` - [`ContractDeclaration`]s to compile. Use [`find_contracts`] to find contracts in
///   `db`.
/// * `compiler_config` - The compiler configuration.
/// # Returns
/// * `Ok(Vec<ContractClass>)` - List of all compiled contract classes found in main crates.
/// * `Err(anyhow::Error)` - Compilation failed.
pub fn compile_prepared_db<'db>(
    db: &'db dyn CloneableDatabase,
    contracts: &[&ContractDeclaration<'db>],
    mut compiler_config: CompilerConfig<'_>,
) -> Result<Vec<ContractClass>> {
    if !contracts.is_empty() {
        let crate_ids: Vec<CrateId<'db>> = contracts
            .iter()
            .map(|c| ModuleId::Submodule(c.submodule_id).owning_crate(db))
            .unique()
            .collect();
        let modules: Vec<ModuleId<'db>> = crate_ids
            .iter()
            .flat_map(|cid| db.crate_modules(*cid).iter().copied())
            .collect();
        prewarm_modules_items_parallel(db, &modules);
        if modules.len() > 1 {
            modules
                .par_iter()
                .for_each_with(db.dyn_clone(), |db, module_id| {
                    let db_ref: &dyn CloneableDatabase = db.as_ref();
                    if let Ok(files) = db_ref.module_files(*module_id) {
                        for file_id in files.iter().copied() {
                            let _ = db_ref.file_syntax_diagnostics(file_id);
                        }
                    }
                    let _ = db_ref.module_semantic_diagnostics(*module_id);
                    let _ = db_ref.module_lowering_diagnostics(*module_id);
                });
        }
    }
    ensure_diagnostics(db, &mut compiler_config.diagnostics_reporter)?;
    compile_prepared_db_without_diagnostics(db, contracts, compiler_config)
}

fn prewarm_modules_items_parallel<'db>(
    db: &'db dyn CloneableDatabase,
    modules: &[ModuleId<'db>],
) {
    let mut items: Vec<ModuleItemId<'db>> = Vec::new();
    for &module_id in modules {
        if let Ok(module_data) = module_id.module_data(db) {
            items.extend(module_data.items(db).iter().copied());
        }
    }
    if items.len() > 1 {
        items
            .par_iter()
            .for_each_with(db.dyn_clone(), |db, item| {
                let db_ref: &dyn CloneableDatabase = db.as_ref();
                match item {
                    ModuleItemId::Impl(impl_def_id) => {
                        let _ = db_ref.impl_semantic_declaration_diagnostics(*impl_def_id);
                        let _ = db_ref.impl_functions(*impl_def_id);
                    }
                    ModuleItemId::Trait(trait_id) => {
                        let _ = db_ref.trait_semantic_declaration_diagnostics(*trait_id);
                        let _ = db_ref.trait_functions(*trait_id);
                    }
                    ModuleItemId::FreeFunction(free_fn) => {
                        let _ = db_ref.free_function_declaration_diagnostics(*free_fn);
                    }
                    ModuleItemId::Struct(struct_id) => {
                        let _ = db_ref.struct_declaration_diagnostics(*struct_id);
                        let _ = db_ref.struct_definition_diagnostics(*struct_id);
                    }
                    ModuleItemId::Enum(enum_id) => {
                        let _ = db_ref.enum_declaration_diagnostics(*enum_id);
                        let _ = db_ref.enum_definition_diagnostics(*enum_id);
                    }
                    _ => {}
                }
            });
    }
    let mut funcs: Vec<DefsFunctionWithBodyId<'db>> = Vec::new();
    for &item in &items {
        match item {
            ModuleItemId::FreeFunction(free_fn) => {
                funcs.push(DefsFunctionWithBodyId::Free(free_fn));
            }
            ModuleItemId::Impl(impl_def_id) => {
                if let Ok(impl_funcs) = db.impl_functions(impl_def_id) {
                    for &impl_func in impl_funcs.values() {
                        funcs.push(DefsFunctionWithBodyId::Impl(impl_func));
                    }
                }
            }
            ModuleItemId::Trait(trait_id) => {
                if let Ok(trait_funcs) = db.trait_functions(trait_id) {
                    for &trait_func in trait_funcs.values() {
                        if matches!(db.trait_function_body(trait_func), Ok(Some(_))) {
                            funcs.push(DefsFunctionWithBodyId::Trait(trait_func));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if funcs.len() > 1 {
        funcs
            .par_iter()
            .for_each_with(db.dyn_clone(), |db, func_id| {
                let db_ref: &dyn CloneableDatabase = db.as_ref();
                match func_id {
                    DefsFunctionWithBodyId::Free(f) => {
                        let _ = db_ref.free_function_body_diagnostics(*f);
                    }
                    DefsFunctionWithBodyId::Impl(f) => {
                        let _ = db_ref.impl_function_declaration_diagnostics(*f);
                        let _ = db_ref.impl_function_body_diagnostics(*f);
                    }
                    DefsFunctionWithBodyId::Trait(f) => {
                        let _ = db_ref.trait_function_declaration_diagnostics(*f);
                        let _ = db_ref.trait_function_body_diagnostics(*f);
                    }
                }
            });
        funcs
            .par_iter()
            .for_each_with(db.dyn_clone(), |db, func_id| {
                let db_ref: &dyn CloneableDatabase = db.as_ref();
                let _ = db_ref.semantic_function_with_body_lowering_diagnostics(*func_id);
            });
    }
}

/// Runs Starknet contract compiler checking diagnostics only for the scoped modules of `contracts`.
/// If any scoped module has a syntax, semantic, or lowering error, falls back to `ensure_diagnostics`
/// to emit full formatted diagnostics.
fn collect_contract_modules_under<'db>(
    db: &'db dyn CloneableDatabase,
    modules: &mut Vec<ModuleId<'db>>,
    module_id: ModuleId<'db>,
) {
    modules.push(module_id);
    if let Ok(submodule_ids) = db.module_submodules_ids(module_id) {
        for submodule_id in submodule_ids.iter().copied() {
            collect_contract_modules_under(db, modules, ModuleId::Submodule(submodule_id));
        }
    }
}

pub fn compile_prepared_db_scoped<'db>(
    db: &'db dyn CloneableDatabase,
    contracts: &[&ContractDeclaration<'db>],
    mut compiler_config: CompilerConfig<'_>,
) -> Result<Vec<ContractClass>> {
    let mut scoped_modules: Vec<ModuleId<'db>> = Vec::with_capacity(contracts.len() * 6);
    for contract in contracts {
        collect_contract_modules_under(
            db,
            &mut scoped_modules,
            ModuleId::Submodule(contract.submodule_id),
        );
    }
    scoped_modules = scoped_modules.into_iter().unique().collect();
    prewarm_modules_items_parallel(db, &scoped_modules);

    let has_error = if scoped_modules.len() > 1 {
        scoped_modules
            .par_iter()
            .map_with(db.dyn_clone(), |db, module_id| {
                let db_ref: &dyn CloneableDatabase = db.as_ref();
                if let Ok(files) = db_ref.module_files(*module_id) {
                    for file_id in files.iter().copied() {
                        if db_ref.file_syntax_diagnostics(file_id).check_error_free().is_err() {
                            return true;
                        }
                    }
                }
                if db_ref
                    .module_semantic_diagnostics(*module_id)
                    .map_or(true, |g| g.check_error_free().is_err())
                {
                    return true;
                }
                if db_ref
                    .module_lowering_diagnostics(*module_id)
                    .map_or(true, |g| g.check_error_free().is_err())
                {
                    return true;
                }
                false
            })
            .any(|err| err)
    } else {
        scoped_modules.iter().any(|module_id| {
            if let Ok(files) = db.module_files(*module_id) {
                for file_id in files.iter().copied() {
                    if db.file_syntax_diagnostics(file_id).check_error_free().is_err() {
                        return true;
                    }
                }
            }
            if db
                .module_semantic_diagnostics(*module_id)
                .map_or(true, |g| g.check_error_free().is_err())
            {
                return true;
            }
            if db
                .module_lowering_diagnostics(*module_id)
                .map_or(true, |g| g.check_error_free().is_err())
            {
                return true;
            }
            false
        })
    };

    if has_error {
        ensure_diagnostics(db, &mut compiler_config.diagnostics_reporter)?;
        anyhow::bail!("Compilation failed.");
    }

    compile_prepared_db_without_diagnostics(db, contracts, compiler_config)
}

/// Runs Starknet contract compiler without the preflight global diagnostics walk.
///
/// Callers must only use this when diagnostics were already validated for the
/// unchanged prepared database and contract set.
pub fn compile_prepared_db_without_diagnostics<'db>(
    db: &'db dyn CloneableDatabase,
    contracts: &[&ContractDeclaration<'db>],
    compiler_config: CompilerConfig<'_>,
) -> Result<Vec<ContractClass>> {
    contracts
        .par_iter()
        .map_with(db.dyn_clone(), |db, contract| {
            compile_contract_with_prepared_and_checked_db(db.as_ref(), contract, &compiler_config)
        })
        .collect()
}

/// Compile the declared Starknet contract.
///
/// The `contract` value **must** come from `db`, for example as a result of calling
/// [`find_contracts`]. Does not check diagnostics, it is expected that they are checked
/// by the caller of this function.
fn compile_contract_with_prepared_and_checked_db<'db>(
    db: &'db dyn CloneableDatabase,
    contract: &ContractDeclaration<'db>,
    compiler_config: &CompilerConfig<'_>,
) -> Result<ContractClass> {
    let SemanticEntryPoints { external, l1_handler, constructor } =
        extract_semantic_entrypoints(db, contract)?;
    let requested_function_ids: Vec<ConcreteFunctionWithBodyId<'db>> =
        chain!(&external, &l1_handler, &constructor).map(|f| f.value).collect();
    if requested_function_ids.len() > 1 {
        let mut visited: std::collections::HashSet<ConcreteFunctionWithBodyId<'db>> =
            requested_function_ids.iter().copied().collect();
        let mut frontier: Vec<ConcreteFunctionWithBodyId<'db>> = requested_function_ids.clone();
        while !frontier.is_empty() {
            if frontier.len() > 1 {
                frontier
                    .par_iter()
                    .for_each_with(db.dyn_clone(), |db, &function_id| {
                        let _ = db.as_ref().function_with_body_sierra(function_id);
                    });
            }
            let mut next_frontier = Vec::new();
            for &function_id in &frontier {
                if let Ok(function) = db.function_with_body_sierra(function_id) {
                    for statement in &function.body {
                        if let Some(related_id) =
                            cairo_lang_sierra_generator::program_generator::try_get_function_with_body_id(
                                db,
                                statement,
                            )
                        {
                            if visited.insert(related_id) {
                                next_frontier.push(related_id);
                            }
                        }
                    }
                }
            }
            frontier = next_frontier;
        }
    }
    let SierraProgramWithDebug { program: mut sierra_program, debug_info } = db
        .get_sierra_program_for_functions(requested_function_ids)
        .to_option()
        .context("Compilation failed without any diagnostics.")?
        .clone();

    if compiler_config.replace_ids {
        sierra_program = replace_sierra_ids_in_program(db, &sierra_program);
    }
    let replacer = CanonicalReplacer::from_program(&sierra_program);
    let sierra_program = replacer.apply(&sierra_program);

    let entry_points_by_type = ContractEntryPoints {
        external: get_entry_points(db, &external, &replacer)?,
        l1_handler: get_entry_points(db, &l1_handler, &replacer)?,
        // Later generation of ABI verifies that there is up to one constructor.
        constructor: get_entry_points(db, &constructor, &replacer)?,
    };

    let mut annotations = Annotations::default();

    if compiler_config.add_statements_functions {
        let statements_functions = debug_info.statements_locations.extract_statements_functions(db);
        annotations.extend(Annotations::from(statements_functions))
    };

    if compiler_config.add_statements_code_locations {
        let statements_functions =
            debug_info.statements_locations.extract_statements_source_code_locations(db);
        annotations.extend(Annotations::from(statements_functions))
    };

    if compiler_config.add_functions_debug_info {
        annotations.extend(Annotations::from(
            debug_info.functions_info.extract_serializable_debug_info(db),
        ))
    }

    let abi_builder: Option<AbiBuilder<'db>> =
        AbiBuilder::from_submodule(db, contract.submodule_id, Default::default()).ok();
    let finalized_abi =
        abi_builder.with_context(|| "Unexpected error while generating ABI.")?.finalize();
    let abi = match finalized_abi {
        Ok(abi) => abi,
        Err(e) => anyhow::bail!("Could not create ABI from contract submodule: {}", e),
    };
    let contract_class =
        ContractClass::new(&sierra_program, entry_points_by_type, Some(abi), annotations)?;
    contract_class.sanity_check();
    Ok(contract_class)
}

pub struct SemanticEntryPoints<'db> {
    pub external: Vec<Aliased<ConcreteFunctionWithBodyId<'db>>>,
    pub l1_handler: Vec<Aliased<ConcreteFunctionWithBodyId<'db>>>,
    pub constructor: Vec<Aliased<ConcreteFunctionWithBodyId<'db>>>,
}

/// Extracts functions from the contract.
pub fn extract_semantic_entrypoints<'db>(
    db: &'db dyn Database,
    contract: &ContractDeclaration<'db>,
) -> core::result::Result<SemanticEntryPoints<'db>, anyhow::Error> {
    let external: Vec<_> = get_contract_abi_functions(db, contract, EXTERNAL_MODULE)?
        .into_iter()
        .map(|f| f.map(|f| ConcreteFunctionWithBodyId::from_semantic(db, f)))
        .collect();
    let l1_handler: Vec<_> = get_contract_abi_functions(db, contract, L1_HANDLER_MODULE)?
        .into_iter()
        .map(|f| f.map(|f| ConcreteFunctionWithBodyId::from_semantic(db, f)))
        .collect();
    let constructor: Vec<_> = get_contract_abi_functions(db, contract, CONSTRUCTOR_MODULE)?
        .into_iter()
        .map(|f| f.map(|f| ConcreteFunctionWithBodyId::from_semantic(db, f)))
        .collect();
    if constructor.len() > 1 {
        anyhow::bail!("Expected at most one constructor.");
    }
    Ok(SemanticEntryPoints { external, l1_handler, constructor })
}

/// Returns the entry points given their IDs sorted by selectors.
fn get_entry_points<'db>(
    db: &'db dyn Database,
    entry_point_functions: &[Aliased<ConcreteFunctionWithBodyId<'db>>],
    replacer: &CanonicalReplacer,
) -> Result<Vec<ContractEntryPoint>> {
    let mut entry_points = vec![];
    for function_with_body_id in entry_point_functions {
        let (selector, sierra_id) =
            get_selector_and_sierra_function(db, function_with_body_id, replacer);

        entry_points.push(ContractEntryPoint {
            selector: selector.to_biguint(),
            function_idx: sierra_id.id as usize,
        });
    }
    entry_points.sort_by(|a, b| a.selector.cmp(&b.selector));
    Ok(entry_points)
}

/// Compile a Starknet crate (or specific contract in the crate).
pub fn starknet_compile(
    crate_path: PathBuf,
    contract_path: Option<String>,
    config: Option<CompilerConfig<'_>>,
    inlining_strategy: InliningStrategy,
    allowed_libfuncs_list: Option<ListSelector>,
) -> Result<String> {
    let config = config.unwrap_or_default();
    let contract = compile_path(&crate_path, contract_path.as_deref(), config, inlining_strategy)?;
    contract
        .extract_sierra_program(false)?
        .validate_version_compatible(allowed_libfuncs_list.unwrap_or_default())?;
    serde_json::to_string_pretty(&contract).with_context(|| "Serialization failed.")
}
