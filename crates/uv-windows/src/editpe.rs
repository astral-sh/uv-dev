//! Add resources to an existing PE image without using the Windows resource update APIs.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use goblin::pe::PE;
use goblin::pe::data_directories::SIZEOF_DATA_DIRECTORY;
use goblin::pe::header::{SIZEOF_COFF_HEADER, SIZEOF_PE_MAGIC};
use goblin::pe::optional_header::{
    SIZEOF_STANDARD_FIELDS_32, SIZEOF_STANDARD_FIELDS_64, SIZEOF_WINDOWS_FIELDS_32,
    SIZEOF_WINDOWS_FIELDS_64,
};
use goblin::pe::options::ParseOptions;
use goblin::pe::resource::{
    IMAGE_RESOURCE_DATA_IS_DIRECTORY, IMAGE_RESOURCE_MASK, IMAGE_RESOURCE_NAME_IS_STRING,
    ImageResourceDirectory,
};
use goblin::pe::section_table::{
    IMAGE_SCN_CNT_INITIALIZED_DATA, IMAGE_SCN_MEM_READ, SIZEOF_SECTION_TABLE,
};
use goblin::pe::utils::find_offset;
use thiserror::Error;

const RESOURCE_DIRECTORY_INDEX: usize = 2;
const RESOURCE_DIRECTORY_SIZE: usize = 16;
const RESOURCE_ENTRY_SIZE: usize = 8;
const RESOURCE_DATA_ENTRY_SIZE: usize = 16;
const SIZE_OF_IMAGE_OFFSET: usize = 56;
const MAX_RESOURCE_DEPTH: usize = 16;

/// An error encountered while updating a PE image's resource directory.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to parse the PE image")]
    Parse(#[from] goblin::error::Error),
    #[error("Invalid PE image: {0}")]
    InvalidImage(&'static str),
    #[error("Invalid PE resource directory: {0}")]
    InvalidResource(&'static str),
    #[error("The PE image exceeds the format's size limit")]
    TooLarge,
}

/// Write named resources with neutral language IDs into an existing PE image.
///
/// Existing resources are preserved. The updated resource directory is placed in a new section,
/// leaving all original sections and their contents unchanged.
pub fn write_resources(
    image: &[u8],
    resource_type: u32,
    resources: &[(&str, &[u8])],
) -> Result<Vec<u8>, Error> {
    let executable = PE::parse(image)?;
    let optional_header = executable
        .header
        .optional_header
        .ok_or(Error::InvalidImage("missing optional header"))?;

    if optional_header.windows_fields.number_of_rva_and_sizes
        <= u32::try_from(RESOURCE_DIRECTORY_INDEX).map_err(|_| Error::TooLarge)?
    {
        return Err(Error::InvalidImage("missing resource data directory"));
    }

    let section_alignment = optional_header.windows_fields.section_alignment;
    let file_alignment = optional_header.windows_fields.file_alignment;
    if section_alignment == 0 || file_alignment == 0 {
        return Err(Error::InvalidImage("section alignment must not be zero"));
    }

    let mut root = if let Some(directory) = optional_header
        .data_directories
        .get_resource_table()
        .filter(|directory| directory.virtual_address != 0 && directory.size != 0)
    {
        let offset = find_offset(
            directory.virtual_address as usize,
            &executable.sections,
            file_alignment,
            &ParseOptions::default(),
        )
        .ok_or(Error::InvalidResource(
            "resource directory is outside the image",
        ))?;
        let data = read_bytes(image, offset, directory.size as usize)?;
        parse_directory(image, &executable, data, 0, &mut BTreeSet::new())?
    } else {
        ResourceDirectory::default()
    };

    let entry = if let Some(entry) = root
        .entries
        .iter_mut()
        .find(|entry| entry.name == ResourceName::Id(resource_type))
    {
        entry
    } else {
        root.entries.push(ResourceEntry {
            name: ResourceName::Id(resource_type),
            value: ResourceValue::Directory(ResourceDirectory::default()),
        });
        root.entries
            .last_mut()
            .ok_or(Error::InvalidResource("missing resource type entry"))?
    };

    let ResourceValue::Directory(resource_directory) = &mut entry.value else {
        return Err(Error::InvalidResource(
            "resource type entry is not a directory",
        ));
    };

    for (name, data) in resources {
        let value = ResourceValue::Directory(ResourceDirectory {
            header: ImageResourceDirectory::default(),
            entries: vec![ResourceEntry {
                name: ResourceName::Id(0),
                value: ResourceValue::Data(ResourceData {
                    bytes: data.to_vec(),
                    code_page: 0,
                    reserved: 0,
                }),
            }],
        });

        if let Some(entry) = resource_directory
            .entries
            .iter_mut()
            .find(|entry| entry.name == ResourceName::Name((*name).to_owned()))
        {
            entry.value = value;
        } else {
            resource_directory.entries.push(ResourceEntry {
                name: ResourceName::Name((*name).to_owned()),
                value,
            });
        }
    }

    let last_virtual_address = executable
        .sections
        .iter()
        .try_fold(None, |last_address, section| {
            let address = section
                .virtual_address
                .checked_add(section.virtual_size.max(section.size_of_raw_data))
                .ok_or(Error::TooLarge)?;
            Ok::<_, Error>(Some(
                last_address.map_or(address, |last_address: u32| last_address.max(address)),
            ))
        })?
        .ok_or(Error::InvalidImage("image has no sections"))?;
    let virtual_address = align_to(last_virtual_address, section_alignment)?;

    let mut writer = ResourceWriter::new(virtual_address);
    writer.write_directory(&root)?;
    let resource_data = writer.finish()?;
    let resource_size = u32::try_from(resource_data.len()).map_err(|_| Error::TooLarge)?;
    let raw_size = align_to(resource_size, file_alignment)?;

    let last_raw_end = executable
        .sections
        .iter()
        .filter(|section| section.size_of_raw_data != 0)
        .try_fold(None, |last_end, section| {
            let end = section
                .pointer_to_raw_data
                .checked_add(section.size_of_raw_data)
                .ok_or(Error::TooLarge)?;
            Ok::<_, Error>(Some(
                last_end.map_or(end, |last_end: u32| last_end.max(end)),
            ))
        })?
        .ok_or(Error::InvalidImage("image has no section contents"))?;
    let first_raw_offset = executable
        .sections
        .iter()
        .filter(|section| section.pointer_to_raw_data != 0)
        .map(|section| section.pointer_to_raw_data)
        .min()
        .ok_or(Error::InvalidImage("image has no section contents"))?;
    let raw_offset = align_to(last_raw_end, file_alignment)?;

    let pe_offset = executable.header.dos_header.pe_pointer as usize;
    let optional_offset = pe_offset
        .checked_add(SIZEOF_PE_MAGIC + SIZEOF_COFF_HEADER)
        .ok_or(Error::TooLarge)?;
    let sections_offset = optional_offset
        .checked_add(executable.header.coff_header.size_of_optional_header as usize)
        .ok_or(Error::TooLarge)?;
    let section_count = executable.header.coff_header.number_of_sections;
    let new_section_offset = sections_offset
        .checked_add(usize::from(section_count) * SIZEOF_SECTION_TABLE)
        .ok_or(Error::TooLarge)?;
    let new_section_end = new_section_offset
        .checked_add(SIZEOF_SECTION_TABLE)
        .ok_or(Error::TooLarge)?;
    if new_section_end > first_raw_offset as usize {
        return Err(Error::InvalidImage("no room for another section header"));
    }

    let data_directories_offset = if executable.is_64 {
        SIZEOF_STANDARD_FIELDS_64 + SIZEOF_WINDOWS_FIELDS_64
    } else {
        SIZEOF_STANDARD_FIELDS_32 + SIZEOF_WINDOWS_FIELDS_32
    };
    let directory_offset = optional_offset
        .checked_add(data_directories_offset)
        .and_then(|offset| offset.checked_add(RESOURCE_DIRECTORY_INDEX * SIZEOF_DATA_DIRECTORY))
        .ok_or(Error::TooLarge)?;
    let directory_end = directory_offset
        .checked_add(SIZEOF_DATA_DIRECTORY)
        .ok_or(Error::TooLarge)?;
    if directory_end > sections_offset {
        return Err(Error::InvalidImage(
            "resource data directory exceeds the header",
        ));
    }

    let image_size = align_to(
        virtual_address
            .checked_add(resource_size)
            .ok_or(Error::TooLarge)?,
        section_alignment,
    )?;
    let original_section_end = last_raw_end as usize;
    let original_sections = image
        .get(..original_section_end)
        .ok_or(Error::InvalidImage("section contents exceed the image"))?;
    let original_overlay = image
        .get(original_section_end..)
        .ok_or(Error::InvalidImage("section contents exceed the image"))?;

    let mut output = original_sections.to_vec();
    output.resize(raw_offset as usize, 0);
    output.extend_from_slice(&resource_data);
    output.resize(
        (raw_offset.checked_add(raw_size).ok_or(Error::TooLarge)?) as usize,
        0,
    );
    output.extend_from_slice(original_overlay);

    write_u16(
        &mut output,
        pe_offset + SIZEOF_PE_MAGIC + 2,
        section_count.checked_add(1).ok_or(Error::TooLarge)?,
    )?;
    write_u32(
        &mut output,
        optional_offset + SIZE_OF_IMAGE_OFFSET,
        image_size,
    )?;
    write_u32(&mut output, directory_offset, virtual_address)?;
    write_u32(&mut output, directory_offset + 4, resource_size)?;

    let section = output
        .get_mut(new_section_offset..new_section_end)
        .ok_or(Error::InvalidImage("section header exceeds the image"))?;
    section.fill(0);
    section[..8].copy_from_slice(b".uvrsrc\0");
    write_u32(section, 8, resource_size)?;
    write_u32(section, 12, virtual_address)?;
    write_u32(section, 16, raw_size)?;
    write_u32(section, 20, raw_offset)?;
    write_u32(
        section,
        36,
        IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ,
    )?;

    Ok(output)
}

#[derive(Debug, Default)]
struct ResourceDirectory {
    header: ImageResourceDirectory,
    entries: Vec<ResourceEntry>,
}

#[derive(Debug)]
struct ResourceEntry {
    name: ResourceName,
    value: ResourceValue,
}

#[derive(Debug, Eq, PartialEq)]
enum ResourceName {
    Name(String),
    Id(u32),
}

#[derive(Debug)]
enum ResourceValue {
    Directory(ResourceDirectory),
    Data(ResourceData),
}

#[derive(Debug)]
struct ResourceData {
    bytes: Vec<u8>,
    code_page: u32,
    reserved: u32,
}

fn parse_directory(
    image: &[u8],
    executable: &PE<'_>,
    resource_data: &[u8],
    offset: usize,
    parents: &mut BTreeSet<usize>,
) -> Result<ResourceDirectory, Error> {
    if parents.len() >= MAX_RESOURCE_DEPTH || !parents.insert(offset) {
        return Err(Error::InvalidResource(
            "resource directory contains a cycle",
        ));
    }

    let header = ImageResourceDirectory {
        characteristics: read_u32(resource_data, offset)?,
        time_date_stamp: read_u32(resource_data, offset + 4)?,
        major_version: read_u16(resource_data, offset + 8)?,
        minor_version: read_u16(resource_data, offset + 10)?,
        number_of_named_entries: read_u16(resource_data, offset + 12)?,
        number_of_id_entries: read_u16(resource_data, offset + 14)?,
    };
    let entry_count = usize::from(header.number_of_named_entries)
        .checked_add(usize::from(header.number_of_id_entries))
        .ok_or(Error::TooLarge)?;
    let mut entries = Vec::with_capacity(entry_count);

    for index in 0..entry_count {
        let entry_offset = offset
            .checked_add(RESOURCE_DIRECTORY_SIZE)
            .and_then(|offset| offset.checked_add(index * RESOURCE_ENTRY_SIZE))
            .ok_or(Error::TooLarge)?;
        let name_or_id = read_u32(resource_data, entry_offset)?;
        let value_offset = read_u32(resource_data, entry_offset + 4)?;

        let name = if name_or_id & IMAGE_RESOURCE_NAME_IS_STRING != 0 {
            let string_offset = (name_or_id & IMAGE_RESOURCE_MASK) as usize;
            let length = usize::from(read_u16(resource_data, string_offset)?);
            let bytes = read_bytes(resource_data, string_offset + 2, length * 2)?;
            let units = bytes
                .chunks_exact(2)
                .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
                .collect::<Vec<_>>();
            ResourceName::Name(
                String::from_utf16(&units)
                    .map_err(|_| Error::InvalidResource("resource name is not valid UTF-16"))?,
            )
        } else {
            ResourceName::Id(name_or_id)
        };

        let value = if value_offset & IMAGE_RESOURCE_DATA_IS_DIRECTORY != 0 {
            ResourceValue::Directory(parse_directory(
                image,
                executable,
                resource_data,
                (value_offset & IMAGE_RESOURCE_MASK) as usize,
                parents,
            )?)
        } else {
            let data_offset = value_offset as usize;
            let virtual_address = read_u32(resource_data, data_offset)?;
            let size = read_u32(resource_data, data_offset + 4)? as usize;
            let code_page = read_u32(resource_data, data_offset + 8)?;
            let reserved = read_u32(resource_data, data_offset + 12)?;
            let bytes = if size == 0 {
                Vec::new()
            } else {
                let optional_header = executable
                    .header
                    .optional_header
                    .ok_or(Error::InvalidImage("missing optional header"))?;
                let offset = find_offset(
                    virtual_address as usize,
                    &executable.sections,
                    optional_header.windows_fields.file_alignment,
                    &ParseOptions::default(),
                )
                .ok_or(Error::InvalidResource(
                    "resource contents are outside the image",
                ))?;
                read_bytes(image, offset, size)?.to_vec()
            };
            ResourceValue::Data(ResourceData {
                bytes,
                code_page,
                reserved,
            })
        };

        entries.push(ResourceEntry { name, value });
    }

    parents.remove(&offset);
    Ok(ResourceDirectory { header, entries })
}

struct ResourceWriter<'a> {
    bytes: Vec<u8>,
    virtual_address: u32,
    names: Vec<(usize, &'a str)>,
    data: Vec<(usize, &'a ResourceData)>,
}

impl<'a> ResourceWriter<'a> {
    fn new(virtual_address: u32) -> Self {
        Self {
            bytes: Vec::new(),
            virtual_address,
            names: Vec::new(),
            data: Vec::new(),
        }
    }

    fn write_directory(&mut self, directory: &'a ResourceDirectory) -> Result<u32, Error> {
        let offset = resource_offset(self.bytes.len())?;
        let entries_size = directory
            .entries
            .len()
            .checked_mul(RESOURCE_ENTRY_SIZE)
            .ok_or(Error::TooLarge)?;
        let directory_size = RESOURCE_DIRECTORY_SIZE
            .checked_add(entries_size)
            .ok_or(Error::TooLarge)?;
        self.bytes.resize(
            self.bytes
                .len()
                .checked_add(directory_size)
                .ok_or(Error::TooLarge)?,
            0,
        );

        let named_count = directory
            .entries
            .iter()
            .filter(|entry| matches!(entry.name, ResourceName::Name(_)))
            .count();
        let id_count = directory.entries.len() - named_count;
        write_u32(
            &mut self.bytes,
            offset as usize,
            directory.header.characteristics,
        )?;
        write_u32(
            &mut self.bytes,
            offset as usize + 4,
            directory.header.time_date_stamp,
        )?;
        write_u16(
            &mut self.bytes,
            offset as usize + 8,
            directory.header.major_version,
        )?;
        write_u16(
            &mut self.bytes,
            offset as usize + 10,
            directory.header.minor_version,
        )?;
        write_u16(
            &mut self.bytes,
            offset as usize + 12,
            u16::try_from(named_count).map_err(|_| Error::TooLarge)?,
        )?;
        write_u16(
            &mut self.bytes,
            offset as usize + 14,
            u16::try_from(id_count).map_err(|_| Error::TooLarge)?,
        )?;

        let mut entries = directory.entries.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| match (&left.name, &right.name) {
            (ResourceName::Name(left), ResourceName::Name(right)) => left
                .to_uppercase()
                .cmp(&right.to_uppercase())
                .then_with(|| left.cmp(right)),
            (ResourceName::Name(_), ResourceName::Id(_)) => Ordering::Less,
            (ResourceName::Id(_), ResourceName::Name(_)) => Ordering::Greater,
            (ResourceName::Id(left), ResourceName::Id(right)) => left.cmp(right),
        });

        for (index, entry) in entries.into_iter().enumerate() {
            let entry_offset =
                offset as usize + RESOURCE_DIRECTORY_SIZE + index * RESOURCE_ENTRY_SIZE;
            match &entry.name {
                ResourceName::Name(name) => self.names.push((entry_offset, name)),
                ResourceName::Id(id) => write_u32(&mut self.bytes, entry_offset, *id)?,
            }

            match &entry.value {
                ResourceValue::Directory(directory) => {
                    let child_offset = self.write_directory(directory)?;
                    write_u32(
                        &mut self.bytes,
                        entry_offset + 4,
                        child_offset | IMAGE_RESOURCE_DATA_IS_DIRECTORY,
                    )?;
                }
                ResourceValue::Data(data) => self.data.push((entry_offset + 4, data)),
            }
        }

        Ok(offset)
    }

    fn finish(mut self) -> Result<Vec<u8>, Error> {
        for (entry_offset, name) in std::mem::take(&mut self.names) {
            let offset = resource_offset(self.bytes.len())?;
            let units = name.encode_utf16().collect::<Vec<_>>();
            let length = u16::try_from(units.len()).map_err(|_| Error::TooLarge)?;
            write_u32(
                &mut self.bytes,
                entry_offset,
                offset | IMAGE_RESOURCE_NAME_IS_STRING,
            )?;
            self.bytes.extend_from_slice(&length.to_le_bytes());
            for unit in units {
                self.bytes.extend_from_slice(&unit.to_le_bytes());
            }
        }

        self.align()?;
        let mut descriptors = Vec::with_capacity(self.data.len());
        for (entry_offset, data) in std::mem::take(&mut self.data) {
            let offset = resource_offset(self.bytes.len())?;
            write_u32(&mut self.bytes, entry_offset, offset)?;
            self.bytes.resize(
                self.bytes
                    .len()
                    .checked_add(RESOURCE_DATA_ENTRY_SIZE)
                    .ok_or(Error::TooLarge)?,
                0,
            );
            descriptors.push((offset as usize, data));
        }

        for (descriptor_offset, data) in descriptors {
            self.align()?;
            let offset = resource_offset(self.bytes.len())?;
            write_u32(
                &mut self.bytes,
                descriptor_offset,
                self.virtual_address
                    .checked_add(offset)
                    .ok_or(Error::TooLarge)?,
            )?;
            write_u32(
                &mut self.bytes,
                descriptor_offset + 4,
                u32::try_from(data.bytes.len()).map_err(|_| Error::TooLarge)?,
            )?;
            write_u32(&mut self.bytes, descriptor_offset + 8, data.code_page)?;
            write_u32(&mut self.bytes, descriptor_offset + 12, data.reserved)?;
            self.bytes.extend_from_slice(&data.bytes);
        }

        Ok(self.bytes)
    }

    fn align(&mut self) -> Result<(), Error> {
        let length = u32::try_from(self.bytes.len()).map_err(|_| Error::TooLarge)?;
        self.bytes.resize(align_to(length, 4)? as usize, 0);
        Ok(())
    }
}

fn resource_offset(offset: usize) -> Result<u32, Error> {
    let offset = u32::try_from(offset).map_err(|_| Error::TooLarge)?;
    if offset > IMAGE_RESOURCE_MASK {
        return Err(Error::TooLarge);
    }
    Ok(offset)
}

fn align_to(value: u32, alignment: u32) -> Result<u32, Error> {
    if alignment == 0 {
        return Err(Error::InvalidImage("alignment must not be zero"));
    }
    let remainder = value % alignment;
    if remainder == 0 {
        Ok(value)
    } else {
        value
            .checked_add(alignment - remainder)
            .ok_or(Error::TooLarge)
    }
}

fn read_bytes(bytes: &[u8], offset: usize, length: usize) -> Result<&[u8], Error> {
    let end = offset.checked_add(length).ok_or(Error::TooLarge)?;
    bytes.get(offset..end).ok_or(Error::InvalidResource(
        "resource data extends beyond its directory",
    ))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, Error> {
    let mut value = [0; 2];
    value.copy_from_slice(read_bytes(bytes, offset, 2)?);
    Ok(u16::from_le_bytes(value))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    let mut value = [0; 4];
    value.copy_from_slice(read_bytes(bytes, offset, 4)?);
    Ok(u32::from_le_bytes(value))
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) -> Result<(), Error> {
    write_bytes(bytes, offset, &value.to_le_bytes())
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<(), Error> {
    write_bytes(bytes, offset, &value.to_le_bytes())
}

fn write_bytes(bytes: &mut [u8], offset: usize, value: &[u8]) -> Result<(), Error> {
    let end = offset.checked_add(value.len()).ok_or(Error::TooLarge)?;
    let destination = bytes
        .get_mut(offset..end)
        .ok_or(Error::InvalidImage("header write exceeds the image"))?;
    destination.copy_from_slice(value);
    Ok(())
}
