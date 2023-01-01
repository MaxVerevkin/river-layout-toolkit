#![warn(clippy::match_same_arms)]
#![warn(clippy::semicolon_if_nothing_returned)]
#![warn(clippy::unnecessary_wraps)]
#![allow(clippy::single_component_path_imports)]

mod protocol {
    use wayrs_client;
    use wayrs_client::protocol::*;
    wayrs_scanner::generate!("river-layout-v3.xml");
}

use protocol::*;
use wayrs_client::protocol::*;

use wayrs_client::connection::Connection;
use wayrs_client::global::{Global, GlobalExt, GlobalsExt};
use wayrs_client::proxy::{Dispatch, Dispatcher};
use wayrs_client::socket::IoMode;

use std::error::Error as StdError;
use std::ffi::CString;
use std::io;

/// This trait represents a layout generator implementation.
pub trait Layout: 'static {
    /// The error type of [`user_cmd`](Self::user_cmd) and [`generate_layout`](Self::generate_layout)
    /// functions. Use [`Infallible`](std::convert::Infallible) if you don't need it.
    type Error: StdError;

    /// The namespace is used by the compositor to distinguish between layout generators. Two separate
    /// clients may not share a namespace. Otherwise, [`run`] will return [`Error::NamespaceInUse`].
    const NAMESPACE: &'static str;

    /// This function is called whenever the user sends a command via `riverctl send-layout-cmd`.
    fn user_cmd(&mut self, cmd: String, tags: Option<u32>, output: &str)
        -> Result<(), Self::Error>;

    /// This function is called whenever compositor requests a layout.
    fn generate_layout(
        &mut self,
        view_count: u32,
        usable_width: u32,
        usable_height: u32,
        tags: u32,
        output: &str,
    ) -> Result<GeneratedLayout, Self::Error>;
}

#[derive(Debug)]
pub struct GeneratedLayout {
    pub layout_name: String,
    pub views: Vec<Rectangle>,
}

#[derive(Debug)]
pub struct Rectangle {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum Error<E: StdError> {
    #[error("Could not connect to Waylasd: {0}")]
    WaylandConnect(#[from] wayrs_client::ConnectError),
    #[error("Unsupported compositor: {0}")]
    WaylandBind(#[from] wayrs_client::global::BindError),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Namespace '{0}' is in use")]
    NamespaceInUse(String),
    #[error("Invalid generated layout")]
    InvalidGeneratedLayout,
    #[error("Layout error: {0}")]
    LayoutError(E),
}

pub fn run<L: Layout>(layout: L) -> Result<(), Error<L::Error>> {
    let mut conn = Connection::connect()?;
    let globals = conn.blocking_collect_initial_globals()?;

    let layout_manager = globals.bind(&mut conn, 1..=2)?;

    let outputs = globals
        .iter()
        .filter(|g| g.is::<WlOutput>())
        .map(|g| Output::bind(&mut conn, g))
        .collect();

    let mut state = State {
        layout_manager,
        last_user_cmd_tags: None,
        layout,
        outputs,
    };

    loop {
        conn.flush(IoMode::Blocking)?;
        conn.recv_events(IoMode::Blocking)?;
        conn.dispatch_events(&mut state)?;
    }
}

struct State<L: Layout> {
    layout_manager: river_layout_manager_v3::RiverLayoutManagerV3,
    last_user_cmd_tags: Option<u32>,
    layout: L,
    outputs: Vec<Output>,
}

struct Output {
    wl_output: WlOutput,
    reg_name: u32,
    river_layout: Option<RiverLayout>,
}

struct RiverLayout {
    river: RiverLayoutV3,
    output_name: String,
}

impl Output {
    fn bind<L: Layout>(conn: &mut Connection<State<L>>, global: &Global) -> Self {
        Self {
            wl_output: global.bind(conn, 4..=4).unwrap(),
            reg_name: global.name,
            river_layout: None,
        }
    }

    fn drop<L: Layout>(self, conn: &mut Connection<State<L>>) {
        if let Some(river_layout) = self.river_layout {
            river_layout.river.destroy(conn);
        }
        self.wl_output.release(conn);
    }
}

impl<L: Layout> Dispatcher for State<L> {
    type Error = Error<L::Error>;
}

impl<L: Layout> Dispatch<WlRegistry> for State<L> {
    fn event(&mut self, conn: &mut Connection<Self>, _: WlRegistry, event: wl_registry::Event) {
        match event {
            wl_registry::Event::Global(global) if global.is::<WlOutput>() => {
                self.outputs.push(Output::bind(conn, &global));
            }
            wl_registry::Event::GlobalRemove(name) => {
                if let Some(output_index) = self.outputs.iter().position(|o| o.reg_name == name) {
                    let output = self.outputs.swap_remove(output_index);
                    output.drop(conn);
                }
            }
            _ => (),
        }
    }
}

impl<L: Layout> Dispatch<WlOutput> for State<L> {
    fn event(&mut self, conn: &mut Connection<Self>, output: WlOutput, event: wl_output::Event) {
        let output = self
            .outputs
            .iter_mut()
            .find(|o| o.wl_output == output)
            .expect("Received event for unknown output");

        if output.river_layout.is_some() {
            return;
        }

        if let wl_output::Event::Name(name) = event {
            output.river_layout = Some(RiverLayout {
                river: self.layout_manager.get_layout(
                    conn,
                    output.wl_output,
                    CString::new(L::NAMESPACE).unwrap(),
                ),
                output_name: name.into_string().unwrap(),
            });
        }
    }
}

impl<L: Layout> Dispatch<RiverLayoutV3> for State<L> {
    fn try_event(
        &mut self,
        conn: &mut Connection<Self>,
        layout: RiverLayoutV3,
        event: river_layout_v3::Event,
    ) -> Result<(), Error<L::Error>> {
        use river_layout_v3::Event;

        let layout = self
            .outputs
            .iter()
            .filter_map(|o| o.river_layout.as_ref())
            .find(|o| o.river == layout)
            .expect("Received event for unknown layout object");

        match event {
            Event::NamespaceInUse => {
                return Err(Error::NamespaceInUse(L::NAMESPACE.into()));
            }
            Event::LayoutDemand(args) => {
                let generated_layout = self
                    .layout
                    .generate_layout(
                        args.view_count,
                        args.usable_width,
                        args.usable_height,
                        args.tags,
                        &layout.output_name,
                    )
                    .map_err(Error::LayoutError)?;

                if generated_layout.views.len() != args.view_count as usize {
                    return Err(Error::InvalidGeneratedLayout);
                }

                for rect in generated_layout.views {
                    layout.river.push_view_dimensions(
                        conn,
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        args.serial,
                    );
                }

                layout.river.commit(
                    conn,
                    CString::new(generated_layout.layout_name).unwrap(),
                    args.serial,
                );
            }
            Event::UserCommand(command) => {
                if let Err(err) = self.layout.user_cmd(
                    command.into_string().unwrap(),
                    self.last_user_cmd_tags,
                    &layout.output_name,
                ) {
                    return Err(Error::LayoutError(err));
                }
            }
            Event::UserCommandTags(tags) => {
                self.last_user_cmd_tags = Some(tags);
            }
        }

        Ok(())
    }
}

// No events
impl<L: Layout> Dispatch<RiverLayoutManagerV3> for State<L> {}
