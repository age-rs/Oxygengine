use crate::{
    ast::{self, *},
    GUID,
};
use core::prefab::{PrefabNumber, PrefabValue};
use petgraph::{algo::toposort, Direction, Graph};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    rc::Rc,
};

pub type Reference = Rc<RefCell<Value>>;

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum Value {
    None,
    Bool(bool),
    Number(PrefabNumber),
    String(String),
    List(Vec<Reference>),
    Object(BTreeMap<String, Reference>),
}

impl From<PrefabValue> for Value {
    fn from(value: PrefabValue) -> Self {
        match value {
            PrefabValue::Null => Value::None,
            PrefabValue::Bool(v) => Value::Bool(v),
            PrefabValue::Number(v) => Value::Number(v),
            PrefabValue::String(v) => Value::String(v),
            PrefabValue::Sequence(v) => Value::List(
                v.into_iter()
                    .map(|v| Rc::new(RefCell::new(v.into())))
                    .collect(),
            ),
            PrefabValue::Mapping(v) => Value::Object(
                v.into_iter()
                    .map(|(k, v)| {
                        // TODO: return error instead of panicking.
                        let k = if let PrefabValue::String(k) = k {
                            k
                        } else {
                            panic!("Mapping key is not a string: {:?}", k);
                        };
                        let v = Rc::new(RefCell::new(v.into()));
                        (k, v)
                    })
                    .collect(),
            ),
        }
    }
}

impl Into<PrefabValue> for Value {
    fn into(self) -> PrefabValue {
        match self {
            Value::None => PrefabValue::Null,
            Value::Bool(v) => PrefabValue::Bool(v),
            Value::Number(v) => PrefabValue::Number(v),
            Value::String(v) => PrefabValue::String(v),
            Value::List(v) => PrefabValue::Sequence(
                v.into_iter()
                    .map(|v| v.as_ref().clone().into_inner().into())
                    .collect(),
            ),
            Value::Object(v) => PrefabValue::Mapping(
                v.into_iter()
                    .map(|(k, v)| {
                        let k = PrefabValue::String(k);
                        let v = v.as_ref().clone().into_inner().into();
                        (k, v)
                    })
                    .collect(),
            ),
        }
    }
}

impl Into<Reference> for Value {
    fn into(self) -> Reference {
        Rc::new(RefCell::new(self))
    }
}

#[derive(Debug)]
pub enum VmError {
    Message(String),
    CompilationError(String),
    /// (expected, provided)
    WrongNumberOfInputs(usize, usize),
    /// (expected, provided)
    WrongNumberOfOutputs(usize, usize),
    CouldNotRunEvent(String),
    CouldNotCallFunction(ast::Reference),
    CouldNotCallMethod(ast::Reference, ast::Reference),
    EventDoesNotExists(ast::Reference),
    NodeDoesNotExists(ast::Reference),
    TypeDoesNotExists(ast::Reference),
    TraitDoesNotExists(ast::Reference),
    MethodDoesNotExists(ast::Reference),
    FunctionDoesNotExists(ast::Reference),
    /// (type guid, method guid)
    TypeDoesNotImplementMethod(ast::Reference, ast::Reference),
    InstanceDoesNotExists,
    GlobalVariableDoesNotExists(ast::Reference),
    LocalVariableDoesNotExists(ast::Reference),
    InputDoesNotExists(usize),
    OutputDoesNotExists(usize),
    StackUnderflow,
    OperationDoesNotExists(ast::Reference),
    OperationIsNotRegistered(String),
    /// (expected, provided, list)
    IndexOutOfBounds(usize, usize, Reference),
    ObjectKeyDoesNotExists(String, Reference),
    ValueIsNotAList(Reference),
    ValueIsNotAnObject(Reference),
    ValueIsNotABool(Reference),
    TryingToPerformInvalidNodeType(NodeType),
    /// (source value, destination value)
    TryingToMutateBorrowedReference(Reference, Reference),
    NodeNotFoundInExecutionPipeline(ast::Reference),
    NodeIsNotALoop(ast::Reference),
    NodeIsNotAnIfElse(ast::Reference),
    TryingToBreakIfElse,
    TryingToContinueIfElse,
    ThereAreNoCachedNodeOutputs(ast::Reference),
    ThereIsNoCachedNodeIndexedOutput(Link),
}

#[derive(Debug)]
pub enum VmOperationError {
    CouldNotPerformOperation {
        message: String,
        name: String,
        inputs: Vec<Value>,
    },
}

pub struct Vm {
    ast: Program,
    operations: HashMap<String, Box<dyn VmOperation>>,
    variables: HashMap<GUID, Reference>,
    running_events: HashMap<GUID, VmEvent>,
    completed_events: HashMap<GUID, Vec<Reference>>,
    /// {event guid: [nodes guid]}
    event_execution_order: HashMap<GUID, Vec<GUID>>,
    /// {(type guid, method guid): [nodes guid]}
    method_execution_order: HashMap<(GUID, GUID), Vec<GUID>>,
    /// {event guid: [nodes guid]}
    function_execution_order: HashMap<GUID, Vec<GUID>>,
    /// {type guid: {method guid: (trait guid, is implemented by type)}}
    type_methods: HashMap<GUID, HashMap<GUID, (GUID, bool)>>,
    end_nodes: Vec<GUID>,
}

impl Vm {
    pub fn new(ast: Program) -> Result<Self, VmError> {
        let type_methods = ast
            .types
            .iter()
            .map(|type_| {
                let mut map = HashMap::new();
                for (trait_ref, methods) in &type_.traits_implementation {
                    let trait_ = match trait_ref {
                        ast::Reference::None => None,
                        ast::Reference::Guid(guid) => ast.traits.iter().find(|t| t.guid == *guid),
                        ast::Reference::Named(name) => {
                            ast.traits.iter().find(|t| t.name.as_str() == name)
                        }
                    };
                    if let Some(trait_) = trait_ {
                        for trait_method in &trait_.methods {
                            let b = methods
                                .iter()
                                .any(|m| m.name.as_str() == &trait_method.name);
                            map.insert(trait_method.guid, (trait_.guid, b));
                        }
                    } else {
                        return Err(VmError::TraitDoesNotExists(trait_ref.clone()));
                    }
                }
                Ok((type_.guid, map))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        let mut end_nodes = vec![];
        let event_execution_order = ast
            .events
            .iter()
            .map(|event| {
                let mut graph = Graph::<GUID, ()>::new();
                let nodes_map = event
                    .nodes
                    .iter()
                    .map(|node| (node.guid, graph.add_node(node.guid)))
                    .collect::<HashMap<_, _>>();
                for node in &event.nodes {
                    match &node.next_node {
                        ast::Reference::None => {}
                        ast::Reference::Guid(guid) => {
                            let from = *nodes_map.get(&node.guid).unwrap();
                            let to = *nodes_map.get(&guid).unwrap();
                            graph.add_edge(from, to, ());
                        }
                        ast::Reference::Named(name) => {
                            if let Some(n) = event.nodes.iter().find(|n| n.name.as_str() == name) {
                                let from = *nodes_map.get(&node.guid).unwrap();
                                let to = *nodes_map.get(&n.guid).unwrap();
                                graph.add_edge(from, to, ());
                            } else {
                                return Err(VmError::NodeDoesNotExists(node.next_node.clone()));
                            }
                        }
                    }
                    for link in &node.input_links {
                        match link {
                            Link::NodeIndexed(guid, _) => {
                                let from = *nodes_map.get(&guid).unwrap();
                                let to = *nodes_map.get(&node.guid).unwrap();
                                graph.add_edge(from, to, ());
                            }
                            Link::None => {}
                        }
                    }
                }
                let list = match toposort(&graph, None) {
                    Ok(list) => Ok(list
                        .into_iter()
                        .map(|index| *nodes_map.iter().find(|(_, v)| **v == index).unwrap().0)
                        .collect::<Vec<_>>()),
                    Err(_) => Err(VmError::CompilationError(
                        "Found flow graph to be cyclic".to_owned(),
                    )),
                }?;
                end_nodes.extend(
                    graph
                        .externals(Direction::Outgoing)
                        .map(|index| *nodes_map.iter().find(|(_, v)| **v == index).unwrap().0),
                );
                Ok((event.guid, list))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        let method_execution_order = {
            let mut result = HashMap::new();
            for type_ in &ast.types {
                for (trait_, methods) in &type_.traits_implementation {
                    let trait_ = match trait_ {
                        ast::Reference::None => None,
                        ast::Reference::Guid(guid) => ast.traits.iter().find(|t| t.guid == *guid),
                        ast::Reference::Named(name) => {
                            ast.traits.iter().find(|t| t.name.as_str() == name)
                        }
                    };
                    if let Some(trait_) = trait_ {
                        for method in &trait_.methods {
                            let method = if let Some(method) =
                                methods.iter().find(|m| m.name.as_str() == method.name)
                            {
                                method
                            } else {
                                method
                            };
                            let mut graph = Graph::<GUID, ()>::new();
                            let nodes_map = method
                                .nodes
                                .iter()
                                .map(|node| (node.guid, graph.add_node(node.guid)))
                                .collect::<HashMap<_, _>>();
                            for node in &method.nodes {
                                match &node.next_node {
                                    ast::Reference::None => {}
                                    ast::Reference::Guid(guid) => {
                                        let from = *nodes_map.get(&node.guid).unwrap();
                                        let to = *nodes_map.get(&guid).unwrap();
                                        graph.add_edge(from, to, ());
                                    }
                                    ast::Reference::Named(name) => {
                                        if let Some(n) =
                                            method.nodes.iter().find(|n| n.name.as_str() == name)
                                        {
                                            let from = *nodes_map.get(&node.guid).unwrap();
                                            let to = *nodes_map.get(&n.guid).unwrap();
                                            graph.add_edge(from, to, ());
                                        } else {
                                            return Err(VmError::NodeDoesNotExists(
                                                node.next_node.clone(),
                                            ));
                                        }
                                    }
                                }
                                for link in &node.input_links {
                                    match link {
                                        Link::NodeIndexed(guid, _) => {
                                            let from = *nodes_map.get(&guid).unwrap();
                                            let to = *nodes_map.get(&node.guid).unwrap();
                                            graph.add_edge(from, to, ());
                                        }
                                        Link::None => {}
                                    }
                                }
                            }
                            let list = if let Ok(list) = toposort(&graph, None) {
                                list.into_iter()
                                    .map(|index| {
                                        *nodes_map.iter().find(|(_, v)| **v == index).unwrap().0
                                    })
                                    .collect::<Vec<_>>()
                            } else {
                                return Err(VmError::CompilationError(
                                    "Found flow graph to be cyclic".to_owned(),
                                ));
                            };
                            end_nodes.extend(graph.externals(Direction::Outgoing).map(|index| {
                                *nodes_map.iter().find(|(_, v)| **v == index).unwrap().0
                            }));
                            result.insert((type_.guid, method.guid), list);
                        }
                    }
                }
            }
            result
        };
        let function_execution_order = ast
            .functions
            .iter()
            .map(|function| {
                let mut graph = Graph::<GUID, ()>::new();
                let nodes_map = function
                    .nodes
                    .iter()
                    .map(|node| (node.guid, graph.add_node(node.guid)))
                    .collect::<HashMap<_, _>>();
                for node in &function.nodes {
                    match &node.next_node {
                        ast::Reference::None => {}
                        ast::Reference::Guid(guid) => {
                            let from = *nodes_map.get(&node.guid).unwrap();
                            let to = *nodes_map.get(&guid).unwrap();
                            graph.add_edge(from, to, ());
                        }
                        ast::Reference::Named(name) => {
                            if let Some(n) = function.nodes.iter().find(|n| n.name.as_str() == name)
                            {
                                let from = *nodes_map.get(&node.guid).unwrap();
                                let to = *nodes_map.get(&n.guid).unwrap();
                                graph.add_edge(from, to, ());
                            } else {
                                return Err(VmError::NodeDoesNotExists(node.next_node.clone()));
                            }
                        }
                    }
                    for link in &node.input_links {
                        match link {
                            Link::NodeIndexed(guid, _) => {
                                let from = *nodes_map.get(&guid).unwrap();
                                let to = *nodes_map.get(&node.guid).unwrap();
                                graph.add_edge(from, to, ());
                            }
                            Link::None => {}
                        }
                    }
                }
                let list = match toposort(&graph, None) {
                    Ok(list) => Ok(list
                        .into_iter()
                        .map(|index| *nodes_map.iter().find(|(_, v)| **v == index).unwrap().0)
                        .collect::<Vec<_>>()),
                    Err(_) => Err(VmError::CompilationError(
                        "Found flow graph to be cyclic".to_owned(),
                    )),
                }?;
                end_nodes.extend(
                    graph
                        .externals(Direction::Outgoing)
                        .map(|index| *nodes_map.iter().find(|(_, v)| **v == index).unwrap().0),
                );
                Ok((function.guid, list))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        let variables = ast
            .variables
            .iter()
            .map(|v| (v.guid, Value::None.into()))
            .collect();
        let result = Self {
            ast,
            operations: Default::default(),
            variables,
            running_events: Default::default(),
            completed_events: Default::default(),
            event_execution_order,
            method_execution_order,
            function_execution_order,
            type_methods,
            end_nodes,
        };
        Ok(result)
    }

    pub fn register_operation<T>(&mut self, name: &str, operator: T)
    where
        T: VmOperation + 'static,
    {
        self.operations.insert(name.to_owned(), Box::new(operator));
    }

    pub fn unregister_operator(&mut self, name: &str) -> bool {
        self.operations.remove(name).is_some()
    }

    pub fn global_variable_value(&self, reference: &ast::Reference) -> Result<Reference, VmError> {
        match reference {
            ast::Reference::None => {}
            ast::Reference::Guid(guid) => {
                if let Some(value) = self.variables.get(guid) {
                    return Ok(value.clone());
                }
            }
            ast::Reference::Named(name) => {
                if let Some(variable) = self.ast.variables.iter().find(|v| v.name.as_str() == name)
                {
                    if let Some(value) = self.variables.get(&variable.guid) {
                        return Ok(value.clone());
                    }
                }
            }
        }
        Err(VmError::GlobalVariableDoesNotExists(reference.clone()))
    }

    pub fn set_global_variable_value(
        &mut self,
        reference: &ast::Reference,
        value: Reference,
    ) -> Result<Reference, VmError> {
        match reference {
            ast::Reference::None => {}
            ast::Reference::Guid(guid) => {
                if let Some(v) = self.variables.get_mut(guid) {
                    return Ok(std::mem::replace(v, value));
                }
            }
            ast::Reference::Named(name) => {
                if let Some(variable) = self.ast.variables.iter().find(|v| v.name.as_str() == name)
                {
                    if let Some(v) = self.variables.get_mut(&variable.guid) {
                        return Ok(std::mem::replace(v, value));
                    }
                }
            }
        }
        Err(VmError::GlobalVariableDoesNotExists(reference.clone()))
    }

    pub fn run_event(&mut self, name: &str, inputs: Vec<Reference>) -> Result<GUID, VmError> {
        if let Some(e) = self.ast.events.iter().find(|e| e.name == name) {
            if e.input_constrains.len() != inputs.len() {
                return Err(VmError::WrongNumberOfInputs(
                    e.input_constrains.len(),
                    inputs.len(),
                ));
            }
            let guid = GUID::default();
            match &e.entry_node {
                ast::Reference::None => {
                    self.completed_events.insert(guid, vec![]);
                }
                ast::Reference::Guid(_) | ast::Reference::Named(_) => {
                    if let Some((_, execution)) = self
                        .event_execution_order
                        .iter()
                        .find(|(k, _)| e.guid == **k)
                    {
                        let vars = e.variables.iter().map(|v| v.guid).collect::<Vec<_>>();
                        self.running_events.insert(
                            guid,
                            VmEvent::new(
                                e.guid,
                                execution.clone(),
                                vars,
                                inputs,
                                e.output_constrains.len(),
                            ),
                        );
                    } else {
                        return Err(VmError::CouldNotRunEvent(name.to_owned()));
                    }
                }
            }
            Ok(guid)
        } else {
            Err(VmError::CouldNotRunEvent(name.to_owned()))
        }
    }

    pub fn destroy_event(&mut self, guid: GUID) -> Result<(), VmError> {
        if self.running_events.remove(&guid).is_some() {
            self.completed_events.insert(guid, vec![]);
            Ok(())
        } else {
            Err(VmError::EventDoesNotExists(ast::Reference::Guid(guid)))
        }
    }

    pub fn get_completed_events(&mut self) -> impl Iterator<Item = (GUID, Vec<Reference>)> {
        let map = std::mem::replace(&mut self.completed_events, Default::default());
        map.into_iter().map(|item| item)
    }

    pub fn process_events(&mut self) -> Result<(), VmError> {
        let count = self.running_events.len();
        let events = std::mem::replace(&mut self.running_events, HashMap::with_capacity(count));
        let mut error = None;
        for (key, mut event) in events {
            if error.is_some() {
                self.running_events.insert(key, event);
            } else {
                match self.process_event(&mut event) {
                    Ok(status) => {
                        if status {
                            self.running_events.insert(key, event);
                        } else {
                            self.completed_events.insert(key, event.outputs);
                        }
                    }
                    Err(err) => error = Some(err),
                }
            }
        }
        if let Some(error) = error {
            Err(error)
        } else {
            Ok(())
        }
    }

    pub fn single_step_event(&mut self, guid: GUID) -> Result<(), VmError> {
        if let Some(mut event) = self.running_events.remove(&guid) {
            self.step_event(&mut event)?;
            self.running_events.insert(guid, event);
            Ok(())
        } else {
            Err(VmError::EventDoesNotExists(ast::Reference::Guid(guid)))
        }
    }

    fn step_event(&mut self, event: &mut VmEvent) -> Result<VmStepStatus, VmError> {
        // TODO: try avoid cloning.
        if let Some(node) = event.get_current_node(self).cloned() {
            match &node.node_type {
                NodeType::Halt => {
                    event.go_to_next_node();
                    return Ok(VmStepStatus::Halt);
                }
                NodeType::Loop(loop_) => {
                    event.push_jump_on_stack(VmJump::Loop(node.guid));
                    event.go_to_node(loop_, self)?;
                }
                NodeType::IfElse(if_else) => {
                    let value = event.get_node_output(node.input_links[0])?.clone();
                    let value2 = value.clone();
                    let v = &*value.borrow();
                    if let Value::Bool(v) = v {
                        event.push_jump_on_stack(VmJump::IfElse(node.guid));
                        if *v {
                            event.go_to_node(&if_else.next_node_true, self)?;
                        } else {
                            event.go_to_node(&if_else.next_node_false, self)?;
                        }
                    } else {
                        return Err(VmError::ValueIsNotABool(value2));
                    }
                    drop(v);
                }
                NodeType::Break => match event.pop_jump_from_stack()? {
                    VmJump::Loop(guid) => {
                        let reference = ast::Reference::Guid(guid);
                        let node = event.get_node(&reference, self)?;
                        if let NodeType::Loop(_) = &node.node_type {
                            event.go_to_node(&node.next_node, self)?;
                        } else {
                            return Err(VmError::NodeIsNotALoop(reference));
                        }
                    }
                    VmJump::IfElse(_) => {
                        return Err(VmError::TryingToBreakIfElse);
                    }
                    _ => {}
                },
                NodeType::Continue => match event.pop_jump_from_stack()? {
                    VmJump::Loop(guid) => {
                        let reference = ast::Reference::Guid(guid);
                        let node = event.get_node(&reference, self)?;
                        if let NodeType::Loop(loop_) = &node.node_type {
                            event.go_to_node(&loop_, self)?;
                        } else {
                            return Err(VmError::NodeIsNotALoop(reference));
                        }
                    }
                    VmJump::IfElse(_) => {
                        return Err(VmError::TryingToContinueIfElse);
                    }
                    _ => {}
                },
                NodeType::GetInstance => {
                    let value = event.instance_value()?.clone();
                    event.set_node_output(node.guid, value);
                }
                NodeType::GetGlobalVariable(reference) => {
                    let value = self.global_variable_value(reference)?.clone();
                    event.set_node_output(node.guid, value);
                }
                NodeType::GetLocalVariable(reference) => {
                    let value = event.local_variable_value(reference, self)?.clone();
                    event.set_node_output(node.guid, value);
                }
                NodeType::GetInput(index) => {
                    let value = event.input_value(*index)?.clone();
                    event.set_node_output(node.guid, value);
                }
                NodeType::SetOutput(index) => {
                    let value = event.get_node_output(node.input_links[0])?.clone();
                    event.set_output_value(*index, value)?;
                }
                NodeType::GetValue(value) => {
                    let value: Value = value.data.clone().into();
                    event.set_node_output(node.guid, value.into());
                }
                NodeType::GetListItem(index) => {
                    let value = event.get_node_output(node.input_links[0])?.clone();
                    let value2 = value.clone();
                    let v = &*value.borrow();
                    if let Value::List(list) = v {
                        if let Some(value) = list.get(*index) {
                            event.set_node_output(node.guid, value.clone());
                        } else {
                            return Err(VmError::IndexOutOfBounds(list.len(), *index, value2));
                        }
                    } else {
                        return Err(VmError::ValueIsNotAList(value2));
                    }
                    drop(v);
                }
                NodeType::GetObjectItem(key) => {
                    let value = event.get_node_output(node.input_links[0])?.clone();
                    let value2 = value.clone();
                    let v = &*value.borrow();
                    if let Value::Object(object) = v {
                        if let Some(value) = object.get(key) {
                            event.set_node_output(node.guid, value.clone());
                        } else {
                            return Err(VmError::ObjectKeyDoesNotExists(key.to_owned(), value2));
                        }
                    } else {
                        return Err(VmError::ValueIsNotAnObject(value2));
                    }
                    drop(v);
                }
                NodeType::MutateValue => {
                    let value_dst = event.get_node_output(node.input_links[0])?;
                    let value_dst2 = value_dst.clone();
                    let value_src = event.get_node_output(node.input_links[0])?;
                    let value_src2 = value_src.clone();
                    if let Ok(mut value) = value_dst.try_borrow_mut() {
                        *value = value_src.as_ref().clone().into_inner();
                    } else {
                        return Err(VmError::TryingToMutateBorrowedReference(
                            value_src2, value_dst2,
                        ));
                    }
                    drop(value_dst);
                }
                NodeType::CallOperation(reference) => {
                    if let Some(op) = self.ast.operations.iter().find(|op| match reference {
                        ast::Reference::None => false,
                        ast::Reference::Guid(guid) => op.guid == *guid,
                        ast::Reference::Named(name) => op.name.as_str() == name,
                    }) {
                        if let Some(op_impl) = self.operations.get_mut(&op.name) {
                            let inputs = node
                                .input_links
                                .iter()
                                .map(|link| match event.get_node_output(*link) {
                                    Ok(v) => Ok(v.clone()),
                                    Err(e) => Err(e),
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            if op.input_constrains.len() != inputs.len() {
                                return Err(VmError::WrongNumberOfInputs(
                                    op.input_constrains.len(),
                                    inputs.len(),
                                ));
                            }
                            let outputs = match op_impl.execute(inputs.as_slice()) {
                                Ok(outputs) => outputs,
                                Err(error) => {
                                    return Err(VmError::Message(format!(
                                        "Error during call to {:?} operation: {:?}",
                                        op.name, error
                                    )))
                                }
                            };
                            if op.output_constrains.len() != outputs.len() {
                                return Err(VmError::WrongNumberOfOutputs(
                                    op.output_constrains.len(),
                                    outputs.len(),
                                ));
                            }
                            event.set_node_outputs(node.guid, outputs);
                        } else {
                            return Err(VmError::OperationIsNotRegistered(op.name.clone()));
                        }
                    } else {
                        return Err(VmError::OperationDoesNotExists(reference.clone()));
                    }
                }
                NodeType::CallFunction(reference) => {
                    if let Some(function) = self.ast.functions.iter().find(|f| match reference {
                        ast::Reference::Guid(guid) => f.guid == *guid,
                        ast::Reference::Named(name) => f.name.as_str() == name,
                        ast::Reference::None => false,
                    }) {
                        if let Some((_, execution)) = self
                            .function_execution_order
                            .iter()
                            .find(|(k, _)| function.guid == **k)
                        {
                            let inputs = node
                                .input_links
                                .iter()
                                .map(|link| match event.get_node_output(*link) {
                                    Ok(v) => Ok(v.clone()),
                                    Err(e) => Err(e),
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            if function.input_constrains.len() != inputs.len() {
                                return Err(VmError::WrongNumberOfInputs(
                                    function.input_constrains.len(),
                                    inputs.len(),
                                ));
                            }
                            event.contexts.push(VmContext {
                                owner: VmContextOwner::Function(function.guid),
                                caller_node: Some(node.guid),
                                execution: execution.to_vec(),
                                current: Some(0),
                                instance: None,
                                inputs,
                                outputs: vec![Value::None.into(); function.output_constrains.len()],
                                variables: function
                                    .variables
                                    .iter()
                                    .map(|v| (v.guid, Value::None.into()))
                                    .collect::<HashMap<_, _>>(),
                                jump_stack: vec![VmJump::None(None)],
                                node_outputs: Default::default(),
                            });
                        } else {
                            return Err(VmError::CouldNotCallFunction(reference.clone()));
                        }
                    } else {
                        return Err(VmError::FunctionDoesNotExists(reference.clone()));
                    }
                }
                NodeType::CallMethod(type_ref, method_ref) => {
                    if let Some(type_) = self.ast.types.iter().find(|t| match type_ref {
                        ast::Reference::Guid(guid) => t.guid == *guid,
                        ast::Reference::Named(name) => t.name.as_str() == name,
                        ast::Reference::None => false,
                    }) {
                        let method =
                            type_
                                .traits_implementation
                                .iter()
                                .find_map(|(trait_ref, methods)| {
                                    if let Some(method) =
                                        methods.iter().find(|m| match method_ref {
                                            ast::Reference::Guid(guid) => m.guid == *guid,
                                            ast::Reference::Named(name) => m.name.as_str() == name,
                                            ast::Reference::None => false,
                                        })
                                    {
                                        Some(method)
                                    } else if let Some(trait_) =
                                        self.ast.traits.iter().find(|t| match trait_ref {
                                            ast::Reference::Guid(guid) => t.guid == *guid,
                                            ast::Reference::Named(name) => t.name.as_str() == name,
                                            ast::Reference::None => false,
                                        })
                                    {
                                        trait_.methods.iter().find(|m| match method_ref {
                                            ast::Reference::Guid(guid) => m.guid == *guid,
                                            ast::Reference::Named(name) => m.name.as_str() == name,
                                            ast::Reference::None => false,
                                        })
                                    } else {
                                        None
                                    }
                                });
                        let method = if let Some(method) = method {
                            method
                        } else {
                            return Err(VmError::MethodDoesNotExists(method_ref.clone()));
                        };
                        if let Some((_, execution)) = self
                            .method_execution_order
                            .iter()
                            .find(|((_, k), _)| method.guid == *k)
                        {
                            let inputs = node
                                .input_links
                                .iter()
                                .map(|link| match event.get_node_output(*link) {
                                    Ok(v) => Ok(v.clone()),
                                    Err(e) => Err(e),
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            if method.input_constrains.len() != inputs.len() {
                                return Err(VmError::WrongNumberOfInputs(
                                    method.input_constrains.len(),
                                    inputs.len(),
                                ));
                            }
                            let instance =
                                Some(event.get_node_output(node.input_links[0])?.clone());
                            event.contexts.push(VmContext {
                                owner: VmContextOwner::Method(type_.guid, method.guid),
                                caller_node: Some(node.guid),
                                execution: execution.to_vec(),
                                current: Some(0),
                                instance,
                                inputs,
                                outputs: vec![Value::None.into(); method.output_constrains.len()],
                                variables: method
                                    .variables
                                    .iter()
                                    .map(|v| (v.guid, Value::None.into()))
                                    .collect::<HashMap<_, _>>(),
                                jump_stack: vec![VmJump::None(None)],
                                node_outputs: Default::default(),
                            });
                        } else {
                            return Err(VmError::CouldNotCallMethod(
                                type_ref.clone(),
                                method_ref.clone(),
                            ));
                        }
                    } else {
                        return Err(VmError::TypeDoesNotExists(type_ref.clone()));
                    }
                }
                _ => {
                    return Err(VmError::TryingToPerformInvalidNodeType(
                        node.node_type.clone(),
                    ))
                }
            }
            if self.end_nodes.contains(&node.guid) {
                match event.pop_jump_from_stack()? {
                    VmJump::Loop(guid) => {
                        let reference = ast::Reference::Guid(guid);
                        let node = event.get_node(&reference, self)?;
                        if let NodeType::Loop(_) = &node.node_type {
                            event.go_to_node(&reference, self)?;
                        } else {
                            return Err(VmError::NodeIsNotALoop(reference));
                        }
                    }
                    VmJump::IfElse(guid) => {
                        let reference = ast::Reference::Guid(guid);
                        let node = event.get_node(&reference, self)?;
                        if let NodeType::IfElse(_) = &node.node_type {
                            event.go_to_node(&node.next_node, self)?;
                        } else {
                            return Err(VmError::NodeIsNotAnIfElse(reference));
                        }
                    }
                    _ => {}
                }
            }
            event.go_to_next_node();
            Ok(VmStepStatus::Continue)
        } else {
            Ok(VmStepStatus::Stop)
        }
    }

    fn process_event(&mut self, event: &mut VmEvent) -> Result<bool, VmError> {
        loop {
            match self.step_event(event)? {
                VmStepStatus::Continue => continue,
                VmStepStatus::Halt => return Ok(true),
                VmStepStatus::Stop => break,
            }
        }
        Ok(false)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum VmStepStatus {
    Continue,
    Halt,
    Stop,
}

pub trait VmOperation {
    fn execute(&mut self, inputs: &[Reference]) -> Result<Vec<Reference>, VmOperationError>;
}

#[derive(Debug, Copy, Clone)]
enum VmContextOwner {
    Event(GUID),
    // (type guid, method guid)
    Method(GUID, GUID),
    Function(GUID),
}

#[derive(Debug, Copy, Clone)]
enum VmJump {
    /// calling node guid?
    None(Option<GUID>),
    /// loop node guid
    Loop(GUID),
    /// if-else node guid
    IfElse(GUID),
}

#[derive(Debug, Clone)]
struct VmContext {
    pub owner: VmContextOwner,
    pub caller_node: Option<GUID>,
    pub execution: Vec<GUID>,
    pub current: Option<usize>,
    pub instance: Option<Reference>,
    pub inputs: Vec<Reference>,
    pub outputs: Vec<Reference>,
    pub variables: HashMap<GUID, Reference>,
    pub jump_stack: Vec<VmJump>,
    pub node_outputs: HashMap<GUID, Vec<Reference>>,
}

#[derive(Debug, Clone)]
struct VmEvent {
    pub contexts: Vec<VmContext>,
    pub outputs: Vec<Reference>,
}

impl VmEvent {
    pub fn new(
        owner_event: GUID,
        execution: Vec<GUID>,
        variables: Vec<GUID>,
        inputs: Vec<Reference>,
        outputs: usize,
    ) -> Self {
        Self {
            contexts: vec![VmContext {
                owner: VmContextOwner::Event(owner_event),
                caller_node: None,
                execution,
                current: Some(0),
                instance: None,
                inputs,
                outputs: vec![Value::None.into(); outputs],
                variables: variables
                    .into_iter()
                    .map(|g| (g, Value::None.into()))
                    .collect::<HashMap<_, _>>(),
                jump_stack: vec![VmJump::None(None)],
                node_outputs: Default::default(),
            }],
            outputs: vec![],
        }
    }

    fn get_node_outputs(&self, guid: GUID) -> Result<&[Reference], VmError> {
        if let Some(context) = self.contexts.last() {
            if let Some(outputs) = context.node_outputs.get(&guid) {
                return Ok(outputs);
            }
        }
        Err(VmError::ThereAreNoCachedNodeOutputs(ast::Reference::Guid(
            guid,
        )))
    }

    fn get_node_output(&self, link: Link) -> Result<&Reference, VmError> {
        if let Link::NodeIndexed(guid, index) = link {
            if let Some(output) = self.get_node_outputs(guid)?.get(index) {
                return Ok(output);
            }
        }
        Err(VmError::ThereIsNoCachedNodeIndexedOutput(link))
    }

    fn set_node_outputs(&mut self, guid: GUID, values: Vec<Reference>) {
        if let Some(context) = self.contexts.last_mut() {
            context.node_outputs.insert(guid, values);
        }
    }

    fn set_node_output(&mut self, guid: GUID, value: Reference) {
        self.set_node_outputs(guid, vec![value]);
    }

    fn get_current_node<'a>(&self, vm: &'a Vm) -> Option<&'a Node> {
        if let Some(context) = self.contexts.last() {
            if let Some(current) = context.current {
                if let Some(node_guid) = context.execution.get(current) {
                    if let Ok(node) = self.get_node(&ast::Reference::Guid(*node_guid), vm) {
                        return Some(node);
                    }
                }
            }
        }
        None
    }

    fn get_node<'a>(&self, reference: &ast::Reference, vm: &'a Vm) -> Result<&'a Node, VmError> {
        if let Some(context) = self.contexts.last() {
            match context.owner {
                VmContextOwner::Event(event_guid) => {
                    if let Some(event) = vm.ast.events.iter().find(|e| e.guid == event_guid) {
                        if let Some(node) = event.nodes.iter().find(|n| match reference {
                            ast::Reference::Guid(guid) => n.guid == *guid,
                            ast::Reference::Named(name) => n.name.as_str() == name,
                            ast::Reference::None => false,
                        }) {
                            return Ok(node);
                        }
                    } else {
                        return Err(VmError::EventDoesNotExists(ast::Reference::Guid(
                            event_guid,
                        )));
                    }
                }
                VmContextOwner::Method(type_guid, method_guid) => {
                    if let Some(methods) = vm.type_methods.get(&type_guid) {
                        if let Some((trait_guid, is_impl)) = methods.get(&method_guid) {
                            let type_ = if let Some(type_) =
                                vm.ast.types.iter().find(|t| t.guid == type_guid)
                            {
                                type_
                            } else {
                                return Err(VmError::TypeDoesNotExists(ast::Reference::Guid(
                                    type_guid,
                                )));
                            };
                            if *is_impl {
                                if let Some(method) =
                                    type_.traits_implementation.iter().find_map(|(_, methods)| {
                                        methods.iter().find(|m| m.guid == method_guid)
                                    })
                                {
                                    if let Some(node) =
                                        method.nodes.iter().find(|n| match reference {
                                            ast::Reference::Guid(guid) => n.guid == *guid,
                                            ast::Reference::Named(name) => n.name.as_str() == name,
                                            ast::Reference::None => false,
                                        })
                                    {
                                        return Ok(node);
                                    }
                                } else {
                                    return Err(VmError::MethodDoesNotExists(
                                        ast::Reference::Guid(method_guid),
                                    ));
                                }
                            } else {
                                if let Some(trait_) =
                                    vm.ast.traits.iter().find(|t| t.guid == *trait_guid)
                                {
                                    if let Some(method) =
                                        trait_.methods.iter().find(|m| m.guid == method_guid)
                                    {
                                        if let Some(node) =
                                            method.nodes.iter().find(|n| match reference {
                                                ast::Reference::Guid(guid) => n.guid == *guid,
                                                ast::Reference::Named(name) => {
                                                    n.name.as_str() == name
                                                }
                                                ast::Reference::None => false,
                                            })
                                        {
                                            return Ok(node);
                                        }
                                    } else {
                                        return Err(VmError::MethodDoesNotExists(
                                            ast::Reference::Guid(method_guid),
                                        ));
                                    }
                                } else {
                                    return Err(VmError::TraitDoesNotExists(ast::Reference::Guid(
                                        type_guid,
                                    )));
                                }
                            }
                        } else {
                            return Err(VmError::TypeDoesNotImplementMethod(
                                ast::Reference::Guid(type_guid),
                                ast::Reference::Guid(method_guid),
                            ));
                        }
                    } else {
                        return Err(VmError::NodeDoesNotExists(ast::Reference::Guid(type_guid)));
                    }
                }
                VmContextOwner::Function(function_guid) => {
                    if let Some(function) =
                        vm.ast.functions.iter().find(|f| f.guid == function_guid)
                    {
                        if let Some(node) = function.nodes.iter().find(|n| match reference {
                            ast::Reference::Guid(guid) => n.guid == *guid,
                            ast::Reference::Named(name) => n.name.as_str() == name,
                            ast::Reference::None => false,
                        }) {
                            return Ok(node);
                        }
                    } else {
                        return Err(VmError::FunctionDoesNotExists(ast::Reference::Guid(
                            function_guid,
                        )));
                    }
                }
            }
        }
        Err(VmError::NodeDoesNotExists(reference.clone()))
    }

    fn go_to_node(&mut self, reference: &ast::Reference, vm: &Vm) -> Result<(), VmError> {
        let guid = self.get_node(reference, vm)?.guid;
        if let Some(context) = self.contexts.last() {
            if let Some(index) = context.execution.iter().position(|n| *n == guid) {
                self.contexts.last_mut().unwrap().current = Some(index);
                return Ok(());
            }
        }
        Err(VmError::NodeNotFoundInExecutionPipeline(reference.clone()))
    }

    fn go_to_next_node(&mut self) {
        if let Some(context) = self.contexts.last() {
            if let Some(mut current) = context.current {
                current += 1;
                if current < context.execution.len() {
                    self.contexts.last_mut().unwrap().current = Some(current);
                } else {
                    let context = self.contexts.pop();
                    self.go_to_next_node();
                    if let Some(context) = context {
                        if let Some(caller) = context.caller_node {
                            self.set_node_outputs(caller, context.outputs);
                        } else {
                            self.outputs = context.outputs;
                        }
                    }
                }
            }
        }
    }

    fn push_jump_on_stack(&mut self, jump: VmJump) {
        if let Some(context) = self.contexts.last_mut() {
            context.jump_stack.push(jump);
        }
    }

    fn pop_jump_from_stack(&mut self) -> Result<VmJump, VmError> {
        if let Some(context) = self.contexts.last_mut() {
            if let Some(jump) = context.jump_stack.pop() {
                return Ok(jump);
            }
        }
        Err(VmError::StackUnderflow)
    }

    fn instance_value(&self) -> Result<Reference, VmError> {
        if let Some(context) = self.contexts.last() {
            if let Some(instance) = &context.instance {
                return Ok(instance.clone());
            }
        }
        Err(VmError::InstanceDoesNotExists)
    }

    fn local_variable_value(
        &self,
        reference: &ast::Reference,
        vm: &Vm,
    ) -> Result<Reference, VmError> {
        if let Some(context) = self.contexts.last() {
            match reference {
                ast::Reference::None => {}
                ast::Reference::Guid(guid) => {
                    if let Some(value) = context.variables.get(guid) {
                        return Ok(value.clone());
                    }
                }
                ast::Reference::Named(name) => match context.owner {
                    VmContextOwner::Event(event_guid) => {
                        if let Some(event) = vm.ast.events.iter().find(|e| e.guid == event_guid) {
                            if let Some(variable) =
                                event.variables.iter().find(|v| v.name.as_str() == name)
                            {
                                if let Some(value) = context.variables.get(&variable.guid) {
                                    return Ok(value.clone());
                                }
                            }
                        }
                    }
                    VmContextOwner::Method(type_guid, method_guid) => {
                        if let Some(methods) = vm.type_methods.get(&type_guid) {
                            if let Some((trait_guid, is_impl)) = methods.get(&method_guid) {
                                let type_ = if let Some(type_) =
                                    vm.ast.types.iter().find(|t| t.guid == type_guid)
                                {
                                    type_
                                } else {
                                    return Err(VmError::TypeDoesNotExists(ast::Reference::Guid(
                                        type_guid,
                                    )));
                                };
                                let guid = if *is_impl {
                                    let method = type_.traits_implementation.iter().find_map(
                                        |(_, methods)| {
                                            methods.iter().find(|m| m.name.as_str() == name)
                                        },
                                    );
                                    if let Some(method) = method {
                                        if let Some(variable) = method
                                            .variables
                                            .iter()
                                            .find(|v| v.name.as_str() == name)
                                        {
                                            variable.guid
                                        } else {
                                            return Err(VmError::LocalVariableDoesNotExists(
                                                reference.clone(),
                                            ));
                                        }
                                    } else {
                                        return Err(VmError::MethodDoesNotExists(
                                            ast::Reference::Named(name.to_owned()),
                                        ));
                                    }
                                } else {
                                    if let Some(trait_) =
                                        vm.ast.traits.iter().find(|t| t.guid == *trait_guid)
                                    {
                                        if let Some(method) =
                                            trait_.methods.iter().find(|m| m.guid == method_guid)
                                        {
                                            if let Some(variable) = method
                                                .variables
                                                .iter()
                                                .find(|v| v.name.as_str() == name)
                                            {
                                                variable.guid
                                            } else {
                                                return Err(VmError::LocalVariableDoesNotExists(
                                                    reference.clone(),
                                                ));
                                            }
                                        } else {
                                            return Err(VmError::MethodDoesNotExists(
                                                ast::Reference::Named(name.to_owned()),
                                            ));
                                        }
                                    } else {
                                        return Err(VmError::TraitDoesNotExists(
                                            ast::Reference::Guid(type_guid),
                                        ));
                                    }
                                };
                                if let Some(value) = context.variables.get(&guid) {
                                    return Ok(value.clone());
                                }
                            }
                        }
                    }
                    VmContextOwner::Function(function_guid) => {
                        if let Some(function) =
                            vm.ast.functions.iter().find(|f| f.guid == function_guid)
                        {
                            if let Some(variable) =
                                function.variables.iter().find(|v| v.name.as_str() == name)
                            {
                                if let Some(value) = context.variables.get(&variable.guid) {
                                    return Ok(value.clone());
                                }
                            }
                        }
                    }
                },
            }
        }
        Err(VmError::LocalVariableDoesNotExists(reference.clone()))
    }

    fn input_value(&self, index: usize) -> Result<Reference, VmError> {
        if let Some(context) = self.contexts.last() {
            if let Some(input) = context.inputs.get(index) {
                return Ok(input.clone());
            }
        }
        Err(VmError::InputDoesNotExists(index))
    }

    fn set_output_value(&mut self, index: usize, value: Reference) -> Result<Reference, VmError> {
        if let Some(context) = self.contexts.last_mut() {
            if let Some(output) = context.outputs.get_mut(index) {
                return Ok(std::mem::replace(output, value));
            }
        }
        Err(VmError::OutputDoesNotExists(index))
    }
}