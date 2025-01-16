#![warn(clippy::match_same_arms)]
#![warn(clippy::semicolon_if_nothing_returned)]
#![warn(clippy::unnecessary_wraps)]

use std::error::Error as StdError;
use std::ffi::CString;
use std::io;

use wayrs_client::global::{Global, GlobalExt};
use wayrs_client::protocol::*;
use wayrs_client::{Connection, EventCtx, IoMode};

wayrs_client::generate!("river-layout-v3.xml");

/// This trait represents a layout generator implementation.
pub trait Layout: 'static {
    /// The error type of [`user_cmd`](Self::user_cmd) and [`generate_layout`](Self::generate_layout)
    /// functions. Use [`Infallible`](std::convert::Infallible) if you don't need it.
    type Error: StdError;

    /// The namespace is used by the compositor to distinguish between layout generators. Two separate
    /// clients may not share a namespace. Otherwise, [`run`] will return [`Error::NamespaceInUse`].
    const NAMESPACE: &'static str;

    /// This function is called whenever the user sends a command via `riverctl send-layout-cmd`.
    ///
    /// # Errors
    ///
    /// An error returned from this function will be logged, but it will not terminate the application.
    fn user_cmd(&mut self, cmd: String, tags: Option<u32>, output: &str)
        -> Result<(), Self::Error>;

    /// This function is called whenever compositor requests a layout.
    ///
    /// # Errors
    ///
    /// Returning an error from this fuction will cause [`run`] to terminate.
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
    conn.blocking_roundtrip()?;
    conn.add_registry_cb(wl_registry_cb);

    let mut state = State {
        layout_manager: conn.bind_singleton(1..=2)?,
        last_user_cmd_tags: None,
        layout,
        outputs: Vec::new(),
        error: None,
    };

    loop {
        conn.dispatch_events(&mut state);
        if let Some(err) = state.error.take() {
            return Err(err);
        }

        conn.flush(IoMode::Blocking)?;
        conn.recv_events(IoMode::Blocking)?;
    }
}

struct State<L: Layout> {
    layout_manager: river_layout_manager_v3::RiverLayoutManagerV3,
    last_user_cmd_tags: Option<u32>,
    layout: L,
    outputs: Vec<Output>,
    error: Option<Error<L::Error>>,
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
            wl_output: global.bind_with_cb(conn, 4, wl_output_cb).unwrap(),
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

fn wl_registry_cb<L: Layout>(
    conn: &mut Connection<State<L>>,
    state: &mut State<L>,
    event: &wl_registry::Event,
) {
    match event {
        wl_registry::Event::Global(global) if global.is::<WlOutput>() => {
            state.outputs.push(Output::bind(conn, global));
        }
        wl_registry::Event::GlobalRemove(name) => {
            if let Some(output_index) = state.outputs.iter().position(|o| o.reg_name == *name) {
                let output = state.outputs.swap_remove(output_index);
                output.drop(conn);
            }
        }
        _ => (),
    }
}

fn wl_output_cb<L: Layout>(ctx: EventCtx<State<L>, WlOutput>) {
    let output = ctx
        .state
        .outputs
        .iter_mut()
        .find(|o| o.wl_output == ctx.proxy)
        .expect("Received event for unknown output");

    if output.river_layout.is_some() {
        return;
    }

    if let wl_output::Event::Name(name) = ctx.event {
        output.river_layout = Some(RiverLayout {
            river: ctx.state.layout_manager.get_layout_with_cb(
                ctx.conn,
                output.wl_output,
                CString::new(L::NAMESPACE).unwrap(),
                river_layout_cb,
            ),
            output_name: name.into_string().unwrap(),
        });
    }
}

fn river_layout_cb<L: Layout>(ctx: EventCtx<State<L>, RiverLayoutV3>) {
    use river_layout_v3::Event;

    let layout = ctx
        .state
        .outputs
        .iter()
        .filter_map(|o| o.river_layout.as_ref())
        .find(|o| o.river == ctx.proxy)
        .expect("Received event for unknown layout object");

    match ctx.event {
        Event::NamespaceInUse => {
            ctx.state.error = Some(Error::NamespaceInUse(L::NAMESPACE.into()));
            ctx.conn.break_dispatch_loop();
        }
        Event::LayoutDemand(args) => {
            let generated_layout = match ctx.state.layout.generate_layout(
                args.view_count,
                args.usable_width,
                args.usable_height,
                args.tags,
                &layout.output_name,
            ) {
                Ok(l) => l,
                Err(e) => {
                    ctx.state.error = Some(Error::LayoutError(e));
                    ctx.conn.break_dispatch_loop();
                    return;
                }
            };

            if generated_layout.views.len() != args.view_count as usize {
                ctx.state.error = Some(Error::InvalidGeneratedLayout);
                ctx.conn.break_dispatch_loop();
                return;
            }

            for rect in generated_layout.views {
                layout.river.push_view_dimensions(
                    ctx.conn,
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    args.serial,
                );
            }

            layout.river.commit(
                ctx.conn,
                CString::new(generated_layout.layout_name).unwrap(),
                args.serial,
            );
        }
        Event::UserCommand(command) => {
            if let Err(err) = ctx.state.layout.user_cmd(
                command.into_string().unwrap(),
                ctx.state.last_user_cmd_tags,
                &layout.output_name,
            ) {
                log::warn!("user_cmd error: {err}");
            }
        }
        Event::UserCommandTags(tags) => {
            ctx.state.last_user_cmd_tags = Some(tags);
        }
    }
}
