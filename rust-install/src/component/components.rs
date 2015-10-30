use utils;
use install::InstallPrefix;
use errors::*;

use component::transaction::Transaction;
use component::package::Package;

use std::path::{Path, PathBuf};
use std::fs::File;
use std::io::Write;

pub const COMPONENTS_FILE: &'static str = "components";

pub struct ChangeSet<'a> {
	pub packages: Vec<Box<Package + 'a>>,
	pub to_install: Vec<String>,
	pub to_uninstall: Vec<String>,
}

impl<'a> ChangeSet<'a> {
	pub fn new() -> Self {
		ChangeSet {
			packages: Vec::new(),
			to_install: Vec::new(),
			to_uninstall: Vec::new(),
		}
	}
	pub fn install(&mut self, component: String) {
		self.to_install.push(component);
	}
	pub fn uninstall(&mut self, component: String) {
		self.to_uninstall.push(component);
	}
	pub fn add_package<P: Package + 'a>(&mut self, package: P) {
		self.packages.push(Box::new(package));
	}
}

#[derive(Clone)]
pub struct Components {
	prefix: InstallPrefix,
}

impl Components {
	pub fn new(prefix: InstallPrefix) -> Option<Self> {
		if utils::is_file(prefix.manifest_file(COMPONENTS_FILE)) {
			Some(Components { prefix: prefix })
		} else {
			None
		}
	}
	pub fn init(prefix: InstallPrefix) -> Result<Self> {
		try!(utils::write_file("components", &prefix.manifest_file(COMPONENTS_FILE), ""));
		Ok(Components { prefix: prefix })
	}
	fn rel_components_file(&self) -> String {
		self.prefix.rel_manifest_file(COMPONENTS_FILE)
	}
	fn rel_component_manifest(&self, name: &str) -> String {
		self.prefix.rel_manifest_file(&format!("manifest-{}", name))
	}
	pub fn list(&self) -> Result<Vec<Component>> {
		let path = self.prefix.abs_path(self.rel_components_file());
		let content = try!(utils::read_file("components", &path));
		Ok(content.lines().map(|s| Component {
			components: self.clone(),
			name: s.to_owned(),
		}).collect())
	}
	pub fn add<'a>(&self, name: &str, tx: Transaction<'a>) -> AddingComponent<'a> {
		AddingComponent(ComponentBuilder {
			components: self.clone(),
			name: name.to_owned(),
			parts: Vec::new(),
		}, tx)
	}
	pub fn find(&self, name: &str) -> Result<Option<Component>> {
		let result = try!(self.list());
		Ok(result.into_iter().filter(|c| (c.name() == name)).next())
	}
	pub fn apply_change_set<'a>(&self, change_set: &ChangeSet, default_target: &str, mut tx: Transaction<'a>) -> Result<Transaction<'a>> {
		// First uninstall old packages
		for c in &change_set.to_uninstall {
			let component = try!(try!(self.find(c)).ok_or(Error::InvalidChangeSet));
			tx = try!(component.uninstall(tx));
		}
		// Then install new packages
		let long_suffix = format!("-{}", default_target);
		for c in &change_set.to_install {
			// Compute short name
			let short_name = if c.ends_with(&long_suffix) {
				Some(&c[0..(c.len()-long_suffix.len())])
			} else {
				None
			};
			
			if try!(self.find(c)).is_some() {
				return Err(Error::InvalidChangeSet);
			}
			let p = try!(change_set.packages
				.iter().filter(|p| p.contains(c, short_name)).next()
				.ok_or(Error::InvalidChangeSet));
			
			tx = try!(p.install(self, c, short_name, tx));
		}
		
		Ok(tx)
	}
	pub fn prefix(&self) -> InstallPrefix {
		self.prefix.clone()
	}
}

struct ComponentBuilder {
	components: Components,
	name: String,
	parts: Vec<ComponentPart>,
}

impl ComponentBuilder {
	fn add(&mut self, part: ComponentPart) {
		self.parts.push(part);
	}
	fn finish(self, tx: &mut Transaction) -> Result<Component> {
		// Write component manifest
		let path = self.components.rel_component_manifest(&self.name);
		let abs_path = self.components.prefix.abs_path(&path);
		let mut file = try!(tx.add_file(&self.name, path));
		for part in self.parts {
			try!(writeln!(file, "{}", part.encode())
				.map_err(|e| utils::Error::WritingFile { name: "component", path: abs_path.clone(), error: e }));
		}
		
		// Add component to components file
		let path = self.components.rel_components_file();
		let abs_path = self.components.prefix.abs_path(&path);
		try!(tx.modify_file(path));
		try!(utils::append_file("components", &abs_path, &self.name));
		
		// Done, convert into normal component
		Ok(Component { components: self.components, name: self.name })
	}
}

pub struct AddingComponent<'a>(ComponentBuilder, Transaction<'a>);

impl<'a> AddingComponent<'a> {
	pub fn finish(self) -> Result<(Component, Transaction<'a>)> {
		let AddingComponent(c, mut tx) = self;
		
		let c = try!(c.finish(&mut tx));
		
		Ok((c, tx))
	}
	pub fn add_file(&mut self, path: String) -> Result<File> {
		self.0.add(ComponentPart("file".to_owned(), path.clone()));
		self.1.add_file(&self.0.name, path)
	}
	pub fn add_dir(&mut self, path: String) -> Result<()> {
		self.0.add(ComponentPart("dir".to_owned(), path.clone()));
		self.1.add_dir(&self.0.name, path)
	}
	pub fn copy_file(&mut self, path: String, src: &Path) -> Result<()> {
		self.0.add(ComponentPart("file".to_owned(), path.clone()));
		self.1.copy_file(&self.0.name, path, src)
	}
	pub fn copy_dir(&mut self, path: String, src: &Path) -> Result<()> {
		self.0.add(ComponentPart("dir".to_owned(), path.clone()));
		self.1.copy_dir(&self.0.name, path, src)
	}
}

pub struct ComponentPart(pub String, pub String);

impl ComponentPart {
	pub fn encode(&self) -> String {
		format!("{}:{}", &self.0, &self.1)
	}
	pub fn decode(line: &str) -> Option<Self> {
		line.find(":").map(|pos| ComponentPart(line[0..pos].to_owned(), line[(pos+1)..].to_owned()))
	}
}

#[derive(Clone)]
pub struct Component {
	components: Components,
	name: String,
}

impl Component {
	pub fn manifest_name(&self) -> String {
		format!("manifest-{}", &self.name)
	}
	pub fn manifest_file(&self) -> PathBuf {
		self.components.prefix.manifest_file(&self.manifest_name())
	}
	pub fn rel_manifest_file(&self) -> String {
		self.components.prefix.rel_manifest_file(&self.manifest_name())
	}
	pub fn name(&self) -> &str {
		&self.name
	}
	pub fn parts(&self) -> Result<Vec<ComponentPart>> {
		let mut result = Vec::new();
		for line in try!(utils::read_file("component", &self.manifest_file())).lines() {
			result.push(try!(ComponentPart::decode(line).ok_or_else(|| {
				Error::CorruptComponent(self.name.clone())
			})));
		}
		Ok(result)
	}
	pub fn uninstall<'a>(&self, mut tx: Transaction<'a>) -> Result<Transaction<'a>> {
		// Update components file
		let path = self.components.rel_components_file();
		let abs_path = self.components.prefix.abs_path(&path);
		let temp = try!(tx.temp().new_file());
		try!(utils::filter_file("components", &abs_path, &temp, |l| (l != self.name)));
		try!(tx.modify_file(path));
		try!(utils::rename_file("components", &temp, &abs_path));
		
		// Remove component manifest
		try!(tx.remove_file(self.rel_manifest_file()));
		
		// Remove parts
		for part in try!(self.parts()).into_iter().rev() {
			match &*part.0 {
				"file" => try!(tx.remove_file(part.1)),
				"dir" => try!(tx.remove_dir(part.1)),
				_ => return Err(Error::CorruptComponent(self.name.clone())),
			}
		}
		
		Ok(tx)
	}
}